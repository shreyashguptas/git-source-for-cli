use std::{
    collections::{HashMap, HashSet},
    future::Future,
    io,
    path::{Path, PathBuf},
    pin::Pin,
    time::Duration,
};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{backend::CrosstermBackend, layout::Rect, widgets::ListState, Terminal};
use tokio::sync::mpsc;

use crate::{
    config::{self, Config},
    event::{spawn_event_loop, AppEvent},
    gh::{self, Availability, Pr},
    git::{self, Branch, ChangeKind, Commit, HeadRef, Repo, Status},
    ollama::{self, Availability as OllamaAvailability},
    ui::{
        self,
        panes::{
            commit_input::InputState,
            confirm::{ConfirmAction, ConfirmDialog},
            details::DetailsContent,
            model_picker::ModelPickerState,
        },
    },
};

type BoxedFetch = Pin<Box<dyn Future<Output = Result<String>> + Send>>;
type BoxedOp = Pin<Box<dyn Future<Output = Result<String>> + Send>>;

/// Which pane currently has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Branches,
    Changes,
    Graph,
}

impl Pane {
    pub fn next(self) -> Self {
        match self {
            Pane::Branches => Pane::Changes,
            Pane::Changes => Pane::Graph,
            Pane::Graph => Pane::Branches,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Pane::Branches => Pane::Graph,
            Pane::Changes => Pane::Branches,
            Pane::Graph => Pane::Changes,
        }
    }
}

/// What input mode the app is in. Affects how key events are routed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    /// Editing commit message in the bottom input bar.
    Commit,
}

/// Where each pane was last rendered on screen — used by the mouse handler
/// to hit-test clicks. Updated by the view code on every render.
#[derive(Debug, Clone, Default)]
pub struct PaneRects {
    /// The full main-content area used by the panes (excludes status bar /
    /// commit input / toast). Useful for mouse-drag clamping.
    pub main_area: Rect,
    pub branches: Rect,
    pub changes: Rect,
    pub graph: Rect,
    pub preview: Option<Rect>,
}

/// User-driven width/height overrides for the layout. Stored in absolute
/// columns/rows; clamped to sane mins on each render. `None` means "use the
/// default proportion".
#[derive(Debug, Clone, Default)]
pub struct LayoutOverrides {
    /// Width of the left column (Branches + Changes).
    pub left_width: Option<u16>,
    /// Width of the Graph column (only meaningful when preview is shown).
    pub graph_width: Option<u16>,
    /// Height of the Branches pane within the left column.
    pub branches_height: Option<u16>,
}

/// Which splitter the user is currently dragging, if any.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResizeDrag {
    #[default]
    None,
    /// Vertical splitter between the left column and the Graph column.
    LeftGraph,
    /// Vertical splitter between the Graph and Preview columns.
    GraphPreview,
    /// Horizontal splitter between Branches and Changes inside the left column.
    BranchesChanges,
}

/// Actions exposed as clickable buttons in the Branches pane toolbar.
/// Same dispatch as the keyboard shortcuts (`Enter` / `n` / `p` / `P` / `f` / `m` / `d`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchAction {
    Checkout,
    NewBranch,
    Push,
    Pull,
    Fetch,
    Merge,
    Delete,
}

/// Top-level application state.
pub struct App {
    pub repo: Repo,
    pub events_tx: mpsc::Sender<AppEvent>,

    pub head: HeadRef,
    pub branches: Vec<Branch>,
    pub commits: Vec<Commit>,
    /// Branch whose history is currently shown in the Graph pane. Selection in
    /// Branches changes this source without checking anything out.
    pub graph_branch: Option<String>,
    pub graph_upstream: Option<String>,
    pub graph_root: PathBuf,
    pub graph_request_id: u64,
    pub status: Status,

    pub branches_state: ListState,
    pub changes_state: ListState,
    pub graph_state: ListState,

    pub active_pane: Pane,
    pub input_mode: InputMode,
    pub input: InputState,
    pub size: (u16, u16),
    pub toast: Option<String>,
    pub error: Option<String>,
    pub should_quit: bool,

    pub overlay: Option<DetailsContent>,
    pub overlay_scroll: u16,

    /// Inline live preview (third column).
    pub preview: Option<DetailsContent>,
    pub preview_scroll: u16,
    /// Identity of the thing currently being previewed — sha for commits,
    /// "file:<staged>:<path>" for changes. Used to discard stale fetches.
    pub preview_target: Option<String>,

    pub show_help: bool,

    pub confirm: Option<ConfirmDialog>,

    pub gh: Availability,
    pub prs: HashMap<String, Pr>,
    pub prs_last_refresh: std::time::Instant,

    /// Commits the local HEAD has but origin doesn't (i.e. unpushed).
    pub ahead_shas: HashSet<String>,
    /// Commits origin has but the local HEAD doesn't (i.e. unpulled).
    pub behind_shas: HashSet<String>,

    /// Map of branch name → worktree path. Lets us flag branches that are
    /// checked out in another worktree (which `git checkout` here would refuse).
    pub worktrees: HashMap<String, PathBuf>,

    /// Most recent on-screen pane rectangles, written by `ui::view::render`.
    pub last_rects: PaneRects,

    /// Persisted user config (Ollama settings, etc.). Loaded on startup,
    /// rewritten on user-driven changes (e.g. picking a model).
    pub config: Config,
    /// Outcome of the most recent local-Ollama probe.
    pub ollama: OllamaAvailability,
    /// Active model-picker modal, if open.
    pub model_picker: Option<ModelPickerState>,
    /// Handle to the in-flight generation task — kept so Esc can abort it.
    pub gen_task: Option<tokio::task::JoinHandle<()>>,

    /// User-driven sizing overrides — set by drag, read by the view.
    pub layout_overrides: LayoutOverrides,
    /// Active drag, if any. Set on left-button down over a splitter, cleared on up.
    pub active_drag: ResizeDrag,

    /// On-screen rects of the Branches-pane toolbar buttons, written each
    /// render. Click handler hit-tests these to dispatch the corresponding
    /// `BranchAction`.
    pub branch_button_rects: Vec<(BranchAction, Rect)>,
}

impl App {
    pub async fn new(repo: Repo, events_tx: mpsc::Sender<AppEvent>) -> Result<Self> {
        let head = repo.head().await.unwrap_or(HeadRef::Unborn);
        let graph_branch = match &head {
            HeadRef::Branch(name) => Some(name.clone()),
            _ => None,
        };
        let graph_root = repo.root.clone();
        let config = config::load();
        let mut s = Self {
            repo,
            events_tx,
            head,
            branches: Vec::new(),
            commits: Vec::new(),
            graph_branch,
            graph_upstream: None,
            graph_root,
            graph_request_id: 0,
            status: Status::default(),
            branches_state: ListState::default(),
            changes_state: ListState::default(),
            graph_state: ListState::default(),
            active_pane: Pane::Branches,
            input_mode: InputMode::Normal,
            input: InputState::default(),
            size: (0, 0),
            toast: None,
            error: None,
            should_quit: false,
            overlay: None,
            overlay_scroll: 0,
            preview: None,
            preview_scroll: 0,
            preview_target: None,
            show_help: false,
            confirm: None,
            gh: Availability::NotInstalled,
            prs: HashMap::new(),
            prs_last_refresh: std::time::Instant::now() - Duration::from_secs(3600),
            ahead_shas: HashSet::new(),
            behind_shas: HashSet::new(),
            worktrees: HashMap::new(),
            last_rects: PaneRects::default(),
            config,
            ollama: OllamaAvailability::Unknown,
            model_picker: None,
            gen_task: None,
            layout_overrides: LayoutOverrides::default(),
            active_drag: ResizeDrag::None,
            branch_button_rects: Vec::new(),
        };
        s.branches_state.select(Some(0));
        s.changes_state.select(Some(0));
        s.graph_state.select(Some(0));
        Ok(s)
    }

    pub async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        mut events: mpsc::Receiver<AppEvent>,
    ) -> Result<()> {
        self.spawn_refresh();
        terminal.draw(|f| ui::view::render(self, f))?;

        while !self.should_quit {
            let Some(event) = events.recv().await else {
                break;
            };
            self.handle_event(event).await;
            terminal.draw(|f| ui::view::render(self, f))?;
        }
        Ok(())
    }

    async fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Quit => self.should_quit = true,
            AppEvent::Resize(w, h) => self.size = (w, h),
            AppEvent::Key(key) => self.handle_key(key).await,
            AppEvent::Mouse(m) => self.handle_mouse(m),
            AppEvent::Tick => {
                self.spawn_refresh();
            }
            AppEvent::HeadLoaded(h) => self.head = h,
            AppEvent::BranchesLoaded(b) => {
                clamp_selection(&mut self.branches_state, b.len());
                self.branches = b;
                if self.sync_graph_source_to_branch_selection() {
                    self.spawn_graph_refresh();
                }
            }
            AppEvent::CommitsLoaded {
                request_id,
                commits,
            } => {
                if request_id == self.graph_request_id {
                    clamp_selection(&mut self.graph_state, commits.len());
                    self.commits = commits;
                    self.update_preview();
                }
            }
            AppEvent::StatusLoaded(s) => {
                clamp_selection(&mut self.changes_state, s.files.len());
                self.status = s;
                self.update_preview();
            }
            AppEvent::DivergenceLoaded {
                request_id,
                ahead,
                behind,
            } => {
                if request_id == self.graph_request_id {
                    self.ahead_shas = ahead;
                    self.behind_shas = behind;
                }
            }
            AppEvent::WorktreesLoaded(map) => {
                self.worktrees = map;
                if self.sync_graph_source_to_branch_selection() {
                    self.spawn_graph_refresh();
                }
            }
            AppEvent::OverlayLoaded(c) => {
                if self.overlay.is_some() {
                    self.overlay = Some(c);
                    self.overlay_scroll = 0;
                }
            }
            AppEvent::PreviewLoaded(target, content) => {
                // Only adopt if the user hasn't moved on to a different commit/file.
                if self.preview_target.as_deref() == Some(target.as_str()) {
                    self.preview = Some(content);
                    self.preview_scroll = 0;
                }
            }
            AppEvent::GhAvailability(a) => {
                self.gh = a;
                if self.gh == Availability::Ready {
                    self.spawn_pr_refresh();
                }
            }
            AppEvent::PrsLoaded(map) => {
                self.prs = map;
                self.prs_last_refresh = std::time::Instant::now();
            }
            AppEvent::LoadFailed(msg) => {
                self.toast = Some(msg);
            }
            AppEvent::OllamaAvailability(a) => {
                self.ollama = a;
                // If the user's saved model isn't available, fall back to the
                // first listed model so Ctrl-G "just works" out of the box.
                // We only auto-pick in memory — saving to disk is reserved for
                // explicit user choice via the model picker.
                if let OllamaAvailability::Ready { models } = &self.ollama {
                    let need_pick = self
                        .config
                        .ollama
                        .model
                        .as_ref()
                        .map_or(true, |m| !models.contains(m));
                    if need_pick {
                        if let Some(first) = models.first() {
                            self.config.ollama.model = Some(first.clone());
                        }
                    }
                }
            }
            AppEvent::OllamaToken(tok) => {
                if self.input.generating {
                    self.input.append(&tok);
                }
            }
            AppEvent::OllamaDone(full) => {
                if self.input.generating {
                    self.input.generating = false;
                    self.input.buf = clean_subject(&full);
                    self.input.cursor = self.input.buf.len();
                }
                self.gen_task = None;
            }
            AppEvent::OllamaError(msg) => {
                if self.input.generating || self.input_mode == InputMode::Commit {
                    self.input.clear();
                    self.input_mode = InputMode::Normal;
                }
                self.toast = Some(msg);
                self.gen_task = None;
            }
        }
    }

    async fn handle_key(&mut self, key: KeyEvent) {
        // global Ctrl-C quits unconditionally
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        // Global Ctrl-G — start an Ollama-powered commit-message generation.
        // Works from normal mode and from the commit-input bar (handy for
        // "type `c` then Ctrl-G to autofill"). Suppressed inside modal contexts
        // so users in the picker / help / confirm dialogs don't trigger it
        // accidentally.
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('g') | KeyCode::Char('G'))
        {
            if self.show_help
                || self.confirm.is_some()
                || self.model_picker.is_some()
                || self.overlay.is_some()
            {
                return;
            }
            if self.input.generating {
                return;
            }
            self.start_generation().await;
            return;
        }

        // Model picker has top-level priority once open (after global shortcuts).
        if self.model_picker.is_some() {
            self.handle_model_picker_key(key);
            return;
        }

        // 0. Help overlay: ? toggles, Esc/q closes.
        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.show_help = false;
                }
                _ => {}
            }
            return;
        }

        // 1. Confirmation dialog has highest priority.
        if let Some(dialog) = self.confirm.clone() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm = None;
                    self.run_confirmed_action(dialog.action).await;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.confirm = None;
                }
                _ => {}
            }
            return;
        }

        // 2. Commit-input mode captures most keys.
        if self.input_mode == InputMode::Commit {
            self.handle_commit_input_key(key).await;
            return;
        }

        // 3. Overlay routes navigation but not destructive keys.
        if self.overlay.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.overlay = None;
                    self.overlay_scroll = 0;
                }
                KeyCode::Char('j') | KeyCode::Down => self.scroll_overlay(1),
                KeyCode::Char('k') | KeyCode::Up => self.scroll_overlay(-1),
                KeyCode::PageDown => self.scroll_overlay(15),
                KeyCode::PageUp => self.scroll_overlay(-15),
                KeyCode::Char('g') | KeyCode::Home => self.overlay_scroll = 0,
                KeyCode::Char('G') | KeyCode::End => {
                    if let Some(o) = &self.overlay {
                        self.overlay_scroll = o.line_count().saturating_sub(1) as u16;
                    }
                }
                _ => {}
            }
            return;
        }

        // 4. Normal mode key handling.
        match key.code {
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Tab => {
                self.active_pane = self.active_pane.next();
                if self.active_pane == Pane::Branches {
                    self.refresh_graph_for_selected_branch();
                }
                self.update_preview();
            }
            KeyCode::BackTab => {
                self.active_pane = self.active_pane.prev();
                if self.active_pane == Pane::Branches {
                    self.refresh_graph_for_selected_branch();
                }
                self.update_preview();
            }
            KeyCode::Char('r') => self.spawn_refresh(),
            // `=` resets any drag-resized panes back to default proportions.
            KeyCode::Char('=') => {
                self.layout_overrides = crate::app::LayoutOverrides::default();
                self.toast = Some("layout reset".to_string());
            }
            KeyCode::Char('1') => {
                self.active_pane = Pane::Branches;
                self.refresh_graph_for_selected_branch();
                self.update_preview();
            }
            KeyCode::Char('2') => {
                self.active_pane = Pane::Changes;
                self.update_preview();
            }
            KeyCode::Char('3') => {
                self.active_pane = Pane::Graph;
                self.update_preview();
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('g') | KeyCode::Home => self.move_selection_to(0),
            KeyCode::Char('G') | KeyCode::End => self.move_selection_to(isize::MAX),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::Enter => self.handle_enter().await,

            // Pane-specific write ops
            KeyCode::Char(' ') if self.active_pane == Pane::Changes => {
                self.toggle_stage_selected().await
            }
            KeyCode::Char('a') if self.active_pane == Pane::Changes => self.run_op_async(
                "stage all",
                Box::pin({
                    let r = self.repo.root.clone();
                    async move { git::ops::stage_all(&r).await }
                }),
            ),
            KeyCode::Char('A') if self.active_pane == Pane::Changes => self.run_op_async(
                "unstage all",
                Box::pin({
                    let r = self.repo.root.clone();
                    async move { git::ops::unstage_all(&r).await }
                }),
            ),
            KeyCode::Char('c') if self.active_pane == Pane::Changes => {
                self.input_mode = InputMode::Commit;
                self.input.clear();
            }
            KeyCode::Char('C') if self.active_pane == Pane::Changes => {
                self.input_mode = InputMode::Commit;
                self.input.clear();
                self.input.push_after = true;
            }
            KeyCode::Char('x') if self.active_pane == Pane::Changes => {
                if let Some(idx) = self.changes_state.selected() {
                    if let Some(file) = self.status.files.get(idx) {
                        self.confirm = Some(ConfirmDialog {
                            title: "Discard changes".to_string(),
                            message: format!(
                                "Discard local changes to {}? This cannot be undone.",
                                file.path
                            ),
                            action: ConfirmAction::DiscardFile {
                                path: file.path.clone(),
                            },
                        });
                    }
                }
            }

            // Branches pane
            KeyCode::Char('p') if self.active_pane == Pane::Branches => {
                self.spawn_push();
            }
            KeyCode::Char('P') if self.active_pane == Pane::Branches => self.run_op_async(
                "pull --ff-only",
                Box::pin({
                    let r = self.repo.root.clone();
                    async move { git::ops::pull(&r).await }
                }),
            ),
            KeyCode::Char('f') if self.active_pane == Pane::Branches => self.run_op_async(
                "fetch --all",
                Box::pin({
                    let r = self.repo.root.clone();
                    async move { git::ops::fetch_all(&r).await }
                }),
            ),
            KeyCode::Char('d') if self.active_pane == Pane::Branches => self.confirm_delete(false),
            KeyCode::Char('D') if self.active_pane == Pane::Branches => self.confirm_delete(true),
            KeyCode::Char('n') if self.active_pane == Pane::Branches => self.start_new_branch(),
            KeyCode::Char('m') if self.active_pane == Pane::Branches => self.confirm_merge(),
            KeyCode::Char('o') => self.open_in_browser(),

            // Open the Ollama model picker. Only opens when ollama is reachable
            // and has at least one model available.
            KeyCode::Char('M') => self.open_model_picker(),
            _ => {}
        }
    }

    /// `o`: open the relevant GitHub URL for the current selection.
    /// - Branches pane: open the PR for the selected branch.
    /// - Graph pane: open the commit on github.com (constructed from origin url + sha).
    fn open_in_browser(&mut self) {
        let url = match self.active_pane {
            Pane::Branches => {
                let Some(idx) = self.branches_state.selected() else {
                    return;
                };
                let Some(b) = self.branches.get(idx) else {
                    return;
                };
                match self.prs.get(&b.name) {
                    Some(pr) => Some(pr.url.clone()),
                    None => {
                        self.toast = Some(format!("no PR found for {}", b.name));
                        None
                    }
                }
            }
            Pane::Graph => {
                let Some(idx) = self.graph_state.selected() else {
                    return;
                };
                let Some(commit) = self.commits.get(idx) else {
                    return;
                };
                self.commit_url(&self.graph_root, &commit.hash)
            }
            Pane::Changes => None,
        };
        if let Some(u) = url {
            // open is a thin wrapper over `open` (mac) / `xdg-open` (linux).
            let _ = open::that_detached(&u);
            self.toast = Some(format!("opened {u}"));
        }
    }

    /// Best-effort: derive a github.com commit URL from the `origin` remote.
    fn commit_url(&self, root: &Path, sha: &str) -> Option<String> {
        // We synchronously read `git remote get-url origin` here — it's a one-shot
        // cheap call. If it fails or doesn't look like GitHub, return None.
        let out = std::process::Command::new("git")
            .current_dir(root)
            .args(["remote", "get-url", "origin"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let web = github_web_url(&url)?;
        Some(format!("{web}/commit/{sha}"))
    }

    /// Mouse handling: left-click focuses + selects, scroll wheel scrolls,
    /// drag on a pane border resizes the layout.
    fn handle_mouse(&mut self, ev: MouseEvent) {
        let pos = (ev.column, ev.row);

        // Modal overlays own the scroll wheel.
        let overlay_active = self.show_help || self.confirm.is_some();
        if self.overlay.is_some() {
            match ev.kind {
                MouseEventKind::ScrollUp => self.scroll_overlay(-3),
                MouseEventKind::ScrollDown => self.scroll_overlay(3),
                _ => {}
            }
            return;
        }
        if overlay_active {
            return;
        }

        // Don't redirect mouse while typing a commit message.
        if self.input_mode == InputMode::Commit {
            return;
        }

        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => self.handle_mouse_click(pos),
            MouseEventKind::Drag(MouseButton::Left) => self.handle_mouse_drag(pos),
            MouseEventKind::Up(MouseButton::Left) => {
                self.active_drag = ResizeDrag::None;
            }
            MouseEventKind::ScrollUp => self.handle_mouse_scroll(pos, -3),
            MouseEventKind::ScrollDown => self.handle_mouse_scroll(pos, 3),
            _ => {}
        }
    }

    fn handle_mouse_click(&mut self, (x, y): (u16, u16)) {
        // First: did they grab a splitter? If so, start a drag and don't also
        // select an item.
        if let Some(drag) = self.detect_splitter(x, y) {
            self.active_drag = drag;
            return;
        }

        // Second: did they click a Branches-pane toolbar button?
        if let Some(action) = self.detect_branch_button(x, y) {
            self.run_branch_action(action);
            return;
        }

        let rects = self.last_rects.clone();
        if hit(rects.branches, x, y) {
            self.active_pane = Pane::Branches;
            self.click_select_in_pane(rects.branches, y, Pane::Branches);
            self.refresh_graph_for_selected_branch();
            self.update_preview();
        } else if hit(rects.changes, x, y) {
            self.active_pane = Pane::Changes;
            self.click_select_in_pane(rects.changes, y, Pane::Changes);
            self.update_preview();
        } else if hit(rects.graph, x, y) {
            self.active_pane = Pane::Graph;
            self.click_select_in_pane(rects.graph, y, Pane::Graph);
            self.update_preview();
        } else if let Some(p) = rects.preview {
            if hit(p, x, y) {
                // Preview clicks: no-op (it mirrors Graph/Changes selection).
            }
        }
    }

    fn detect_branch_button(&self, x: u16, y: u16) -> Option<BranchAction> {
        self.branch_button_rects
            .iter()
            .find(|(_, r)| hit(*r, x, y))
            .map(|(a, _)| *a)
    }

    /// Did the click land exactly on a splitter (the actual border line)?
    /// We hit-test on the two adjacent border columns/rows that visually form
    /// the splitter — anything one cell INTO a pane is content, not splitter.
    /// This avoids the bug where clicking the first row of Changes (or the
    /// leftmost col of Graph) was eaten by drag-detection.
    fn detect_splitter(&self, x: u16, y: u16) -> Option<ResizeDrag> {
        let r = &self.last_rects;
        if r.branches.width == 0 {
            return None;
        }

        // Horizontal splitter inside the left column (Branches above Changes).
        // The splitter visually sits on the two adjacent border rows:
        //   - bottom border of Branches (last row of Branches rect)
        //   - top border of Changes (first row of Changes rect)
        if x >= r.branches.x && x < r.branches.x + r.branches.width {
            let branches_bottom_border = r.branches.y + r.branches.height.saturating_sub(1);
            let changes_top_border = r.changes.y;
            if y == branches_bottom_border || y == changes_top_border {
                return Some(ResizeDrag::BranchesChanges);
            }
        }

        // Vertical splitters require y inside the panes' vertical span.
        let v_top = r.branches.y;
        let v_bot = r.graph.y + r.graph.height;
        if y < v_top || y >= v_bot {
            return None;
        }

        // Vertical splitter between Left column and Graph: the rightmost col
        // of Branches/Changes (their right border) and the leftmost col of
        // Graph (its left border) — both render as `│` and are valid hit cols.
        let left_pane_right_border = r.branches.x + r.branches.width.saturating_sub(1);
        let graph_left_border = r.graph.x;
        if x == left_pane_right_border || x == graph_left_border {
            return Some(ResizeDrag::LeftGraph);
        }

        // Vertical splitter between Graph and Preview.
        if let Some(p) = r.preview {
            let graph_right_border = r.graph.x + r.graph.width.saturating_sub(1);
            let preview_left_border = p.x;
            if x == graph_right_border || x == preview_left_border {
                return Some(ResizeDrag::GraphPreview);
            }
        }
        None
    }

    fn handle_mouse_drag(&mut self, (x, y): (u16, u16)) {
        let area = self.last_rects.main_area;
        if area.width == 0 || area.height == 0 {
            return;
        }

        match self.active_drag {
            ResizeDrag::LeftGraph => {
                // Width of the left column = mouse x relative to its left edge.
                let new_w = x.saturating_sub(area.x);
                // Reserve at least 40 cols for graph (+ preview if present).
                let max = area.width.saturating_sub(40);
                self.layout_overrides.left_width = Some(new_w.clamp(15, max.max(15)));
            }
            ResizeDrag::GraphPreview => {
                // Width of graph = mouse x relative to graph's left edge.
                let new_w = x.saturating_sub(self.last_rects.graph.x);
                let left_w = self.last_rects.branches.width;
                let max = area.width.saturating_sub(left_w + 20);
                self.layout_overrides.graph_width = Some(new_w.clamp(20, max.max(20)));
            }
            ResizeDrag::BranchesChanges => {
                // Height of branches pane = mouse y relative to its top.
                let new_h = y.saturating_sub(self.last_rects.branches.y);
                // Leave at least 4 rows for changes pane below.
                let max = area.height.saturating_sub(8);
                self.layout_overrides.branches_height = Some(new_h.clamp(3, max.max(3)));
            }
            ResizeDrag::None => {}
        }
    }

    fn click_select_in_pane(&mut self, rect: Rect, y: u16, pane: Pane) {
        // Inside a bordered Block, the first content row is at rect.y + 1.
        if y < rect.y + 1 || y >= rect.y + rect.height.saturating_sub(1) {
            return; // border row
        }
        let row_in_pane = (y - rect.y - 1) as usize;
        let (state, len) = match pane {
            Pane::Branches => (&mut self.branches_state, self.branches.len()),
            Pane::Changes => (&mut self.changes_state, self.status.files.len()),
            Pane::Graph => (&mut self.graph_state, self.commits.len()),
        };
        let target = state.offset() + row_in_pane;
        if target < len {
            state.select(Some(target));
        }
    }

    fn handle_mouse_scroll(&mut self, (x, y): (u16, u16), delta: isize) {
        let rects = self.last_rects.clone();
        if let Some(p) = rects.preview {
            if hit(p, x, y) {
                // Scroll the preview content directly.
                self.scroll_preview(delta);
                return;
            }
        }
        // Scroll inside list panes = move the selection (visible scroll).
        if hit(rects.branches, x, y) {
            self.active_pane = Pane::Branches;
            self.move_selection(delta);
        } else if hit(rects.changes, x, y) {
            self.active_pane = Pane::Changes;
            self.move_selection(delta);
        } else if hit(rects.graph, x, y) {
            self.active_pane = Pane::Graph;
            self.move_selection(delta);
        } else {
            // Click outside any pane — scroll the active one as a sensible default.
            self.move_selection(delta);
        }
    }

    fn scroll_preview(&mut self, delta: isize) {
        let max = self
            .preview
            .as_ref()
            .map(|c| c.line_count() as isize - 1)
            .unwrap_or(0)
            .max(0);
        let next = (self.preview_scroll as isize + delta).clamp(0, max);
        self.preview_scroll = next as u16;
    }

    async fn handle_commit_input_key(&mut self, key: KeyEvent) {
        // While the model is streaming into the buffer, only Esc is honored
        // (cancel the in-flight generation). Other keys would race the stream.
        if self.input.generating {
            if matches!(key.code, KeyCode::Esc) {
                self.cancel_generation();
                self.input_mode = InputMode::Normal;
                self.input.clear();
                self.toast = Some("generation cancelled".to_string());
            }
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.input.clear();
            }
            KeyCode::Enter => self.do_commit().await,
            KeyCode::Backspace => self.input.backspace(),
            KeyCode::Left => self.input.left(),
            KeyCode::Right => self.input.right(),
            KeyCode::Home => self.input.home(),
            KeyCode::End => self.input.end(),
            KeyCode::Char(c) => {
                // Ignore Ctrl-/Alt-modified char events — they're shortcuts,
                // not literal text. (Plain Shift is fine — produces 'A' etc.)
                if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                    self.input.insert(c);
                }
            }
            _ => {}
        }
    }

    async fn do_commit(&mut self) {
        let raw = self.input.buf.trim().to_string();
        if raw.is_empty() {
            self.toast = Some("aborted: empty input".to_string());
            return;
        }
        let push_after = self.input.push_after;
        self.input_mode = InputMode::Normal;
        self.input.clear();

        // Special case: "branch:<name>" → create + checkout that branch.
        if let Some(name) = raw.strip_prefix("branch:") {
            let name = name.trim().to_string();
            if name.is_empty() {
                self.toast = Some("aborted: empty branch name".to_string());
                return;
            }
            let root = self.repo.root.clone();
            self.run_op_async(
                &format!("create+checkout {name}"),
                Box::pin(async move { git::ops::create_and_checkout(&root, &name).await }),
            );
            return;
        }

        let msg = raw;
        let tx = self.events_tx.clone();
        let root = self.repo.root.clone();
        tokio::spawn(async move {
            match git::ops::commit(&root, &msg).await {
                Ok(s) => {
                    let _ = tx.send(AppEvent::LoadFailed(s)).await;
                    if push_after {
                        match git::ops::push(&root).await {
                            Ok(s) => {
                                let _ = tx.send(AppEvent::LoadFailed(s)).await;
                            }
                            Err(e) => {
                                let _ = tx
                                    .send(AppEvent::LoadFailed(format!("push: {e}")))
                                    .await;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::LoadFailed(format!("commit: {e}"))).await;
                }
            }
        });
    }

    async fn toggle_stage_selected(&mut self) {
        let Some(idx) = self.changes_state.selected() else {
            return;
        };
        let Some(file) = self.status.files.get(idx) else {
            return;
        };
        let path = file.path.clone();
        let staged_already =
            file.staged.is_some() && !matches!(file.kind, ChangeKind::Untracked | ChangeKind::Ignored);
        let label = if staged_already {
            format!("unstage {path}")
        } else {
            format!("stage {path}")
        };
        let root = self.repo.root.clone();
        let op: BoxedOp = if staged_already {
            Box::pin(async move { git::ops::unstage(&root, &path).await })
        } else {
            Box::pin(async move { git::ops::stage(&root, &path).await })
        };
        self.run_op_async(&label, op);
    }

    /// Push current branch — if no upstream, push -u.
    fn spawn_push(&mut self) {
        let branch = match &self.head {
            HeadRef::Branch(b) => b.clone(),
            _ => {
                self.toast = Some("not on a branch — cannot push".to_string());
                return;
            }
        };
        // Look up whether this branch has an upstream.
        let has_upstream = self
            .branches
            .iter()
            .find(|b| b.name == branch)
            .map(|b| b.upstream.is_some())
            .unwrap_or(false);
        let root = self.repo.root.clone();
        let op: BoxedOp = if has_upstream {
            Box::pin(async move { git::ops::push(&root).await })
        } else {
            Box::pin(async move { git::ops::push_set_upstream(&root, &branch).await })
        };
        self.run_op_async("push", op);
    }

    fn confirm_delete(&mut self, force: bool) {
        let Some(idx) = self.branches_state.selected() else {
            return;
        };
        let Some(b) = self.branches.get(idx) else {
            return;
        };
        if b.is_current {
            self.toast = Some("cannot delete current branch — checkout another first".to_string());
            return;
        }
        let title = if force {
            "Force-delete branch"
        } else {
            "Delete branch"
        };
        let message = if force {
            format!(
                "Force-delete branch '{}' (loses unmerged commits)?",
                b.name
            )
        } else {
            format!("Delete branch '{}'?", b.name)
        };
        self.confirm = Some(ConfirmDialog {
            title: title.to_string(),
            message,
            action: ConfirmAction::DeleteBranch {
                name: b.name.clone(),
                force,
            },
        });
    }

    fn confirm_merge(&mut self) {
        let Some(idx) = self.branches_state.selected() else {
            return;
        };
        let Some(b) = self.branches.get(idx) else {
            return;
        };
        if b.is_current {
            self.toast = Some("cannot merge a branch into itself".to_string());
            return;
        }
        let current = match &self.head {
            HeadRef::Branch(s) => s.clone(),
            _ => "(detached)".to_string(),
        };
        self.confirm = Some(ConfirmDialog {
            title: "Merge branch".to_string(),
            message: format!("Merge '{}' into '{}'?", b.name, current),
            action: ConfirmAction::MergeBranch {
                name: b.name.clone(),
            },
        });
    }

    /// Pre-fill commit input with "new branch: " — user types name and Enter creates+checkout.
    fn start_new_branch(&mut self) {
        // Reuse the input bar with a simple convention: the user types a branch name.
        self.input_mode = InputMode::Commit;
        self.input.clear();
        // Hijack: borrow the same input but flag as new-branch via a sentinel prefix.
        // (Cleaner: a separate InputMode variant; deferred for v1 simplicity.)
        self.input.buf = "branch:".to_string();
        self.input.cursor = self.input.buf.len();
        self.toast = Some(
            "type branch name after 'branch:' and press Enter to create+checkout".to_string(),
        );
    }

    async fn run_confirmed_action(&mut self, action: ConfirmAction) {
        let root = self.repo.root.clone();
        match action {
            ConfirmAction::DeleteBranch { name, force } => {
                let label = format!("delete {name}");
                let op: BoxedOp = if force {
                    Box::pin(async move { git::ops::force_delete_branch(&root, &name).await })
                } else {
                    Box::pin(async move { git::ops::delete_branch(&root, &name).await })
                };
                self.run_op_async(&label, op);
            }
            ConfirmAction::DiscardFile { path } => {
                let label = format!("discard {path}");
                let op: BoxedOp =
                    Box::pin(async move { git::ops::discard_file(&root, &path).await });
                self.run_op_async(&label, op);
            }
            ConfirmAction::MergeBranch { name } => {
                let label = format!("merge {name}");
                let op: BoxedOp = Box::pin(async move {
                    git::exec::run(&root, ["merge", "--no-ff", &name])
                        .await
                        .map(|_| format!("merged {name}"))
                });
                self.run_op_async(&label, op);
            }
        }
    }

    /// Open the Ollama model picker modal. No-op (with explanatory toast) if
    /// Ollama isn't reachable or has no models installed.
    fn open_model_picker(&mut self) {
        let models = match &self.ollama {
            OllamaAvailability::Ready { models } => models.clone(),
            OllamaAvailability::NotRunning => {
                self.toast =
                    Some("ollama not running — start it with `ollama serve`".to_string());
                return;
            }
            OllamaAvailability::NoModels => {
                self.toast =
                    Some("ollama running but no models — try `ollama pull qwen2.5-coder`"
                        .to_string());
                return;
            }
            OllamaAvailability::Unknown => {
                self.toast = Some("still detecting ollama, try again in a moment".to_string());
                return;
            }
        };
        let current = self.config.ollama.model.as_deref();
        self.model_picker = Some(ModelPickerState::new(models, current));
    }

    fn handle_model_picker_key(&mut self, key: KeyEvent) {
        let Some(picker) = self.model_picker.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.model_picker = None;
            }
            KeyCode::Char('j') | KeyCode::Down => picker.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => picker.move_selection(-1),
            KeyCode::PageDown => picker.move_selection(10),
            KeyCode::PageUp => picker.move_selection(-10),
            KeyCode::Enter => {
                let picked = picker.selected().map(String::from);
                self.model_picker = None;
                if let Some(name) = picked {
                    self.config.ollama.model = Some(name.clone());
                    match config::save(&self.config) {
                        Ok(()) => {
                            self.toast = Some(format!("model set to {name}"));
                        }
                        Err(e) => {
                            self.toast = Some(format!("model set to {name} (save failed: {e})"));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Kick off a streaming Ollama generation. Pre-flight checks the state
    /// (ollama reachable, model selected, something staged); on success enters
    /// commit-input mode with `generating=true` and spawns the generator.
    async fn start_generation(&mut self) {
        let model = match &self.ollama {
            OllamaAvailability::Ready { models } => self
                .config
                .ollama
                .model
                .clone()
                .filter(|m| models.contains(m))
                .or_else(|| models.first().cloned()),
            OllamaAvailability::NotRunning => {
                self.toast =
                    Some("ollama not running — start it with `ollama serve`".to_string());
                return;
            }
            OllamaAvailability::NoModels => {
                self.toast = Some(
                    "ollama running but no models — try `ollama pull qwen2.5-coder`".to_string(),
                );
                return;
            }
            OllamaAvailability::Unknown => {
                self.toast =
                    Some("still detecting ollama, try again in a moment".to_string());
                return;
            }
        };
        let Some(model) = model else {
            self.toast = Some("ollama has no models available".to_string());
            return;
        };
        let staged_count = self
            .status
            .files
            .iter()
            .filter(|f| {
                f.staged.is_some()
                    && !matches!(f.kind, ChangeKind::Untracked | ChangeKind::Ignored)
            })
            .count();
        if staged_count == 0 {
            self.toast =
                Some("nothing staged — press Space or `a` to stage first".to_string());
            return;
        }

        // Switch the input bar into streaming mode. Preserve `push_after` if
        // the user already opened it via `C` (commit+push).
        let push_after = self.input.push_after;
        self.input_mode = InputMode::Commit;
        self.input.clear();
        self.input.push_after = push_after;
        self.input.generating = true;

        let root = self.repo.root.clone();
        let base_url = self.config.ollama.base_url.clone();
        let system = self.config.ollama.system_prompt.clone();
        let tx = self.events_tx.clone();

        let handle = tokio::spawn(async move {
            // Fetch the staged diff. Empty = nothing to summarize.
            let diff = match git::diff::cached(&root).await {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx
                        .send(AppEvent::OllamaError(format!("git diff: {e}")))
                        .await;
                    return;
                }
            };
            if diff.trim().is_empty() {
                let _ = tx
                    .send(AppEvent::OllamaError("staged diff is empty".to_string()))
                    .await;
                return;
            }
            let (truncated, was_truncated) = truncate_diff(&diff);
            let prompt = if was_truncated {
                format!("Diff (truncated for length):\n\n{truncated}")
            } else {
                format!("Diff:\n\n{truncated}")
            };

            // Forwarder task: shuttles tokens from the generator's mpsc into
            // AppEvents so the UI can append them to the input bar.
            let (tok_tx, mut tok_rx) = mpsc::channel::<String>(64);
            let tx_fwd = tx.clone();
            let forwarder = tokio::spawn(async move {
                while let Some(tok) = tok_rx.recv().await {
                    if tx_fwd.send(AppEvent::OllamaToken(tok)).await.is_err() {
                        break;
                    }
                }
            });

            match ollama::generate_stream(&base_url, &model, &system, &prompt, tok_tx).await {
                Ok(full) => {
                    // Wait for the forwarder to drain so all tokens land before Done.
                    let _ = forwarder.await;
                    let _ = tx.send(AppEvent::OllamaDone(full)).await;
                }
                Err(e) => {
                    forwarder.abort();
                    let _ = tx.send(AppEvent::OllamaError(format!("ollama: {e}"))).await;
                }
            }
        });
        self.gen_task = Some(handle);
    }

    /// Abort any in-flight generation. Safe to call when nothing is running.
    fn cancel_generation(&mut self) {
        if let Some(handle) = self.gen_task.take() {
            handle.abort();
        }
    }

    /// Run a write op in a background task; show toast with result; refresh.
    fn run_op_async(&self, label: &str, op: BoxedOp) {
        let tx = self.events_tx.clone();
        let label = label.to_string();
        tokio::spawn(async move {
            match op.await {
                Ok(s) => {
                    let _ = tx.send(AppEvent::LoadFailed(s)).await; // (re)using toast
                }
                Err(e) => {
                    let _ = tx
                        .send(AppEvent::LoadFailed(format!("{label}: {e}")))
                        .await;
                }
            }
        });
        // Schedule a refresh after a short delay so the post-op state lands quickly.
        let tx2 = self.events_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let _ = tx2.send(AppEvent::Tick).await;
        });
    }

    async fn handle_enter(&mut self) {
        match self.active_pane {
            Pane::Branches => self.checkout_selected_branch(),
            Pane::Changes | Pane::Graph => self.open_details().await,
        }
    }

    /// Checkout the currently selected branch. Synchronous-callable so the
    /// toolbar button can invoke it directly from the (sync) mouse handler.
    fn checkout_selected_branch(&mut self) {
        let Some(idx) = self.branches_state.selected() else {
            return;
        };
        let Some(b) = self.branches.get(idx) else {
            return;
        };
        if b.is_current {
            self.toast = Some(format!("already on '{}'", b.name));
            return;
        }
        if let Some(wt) = self.worktrees.get(&b.name) {
            if wt != &self.repo.root {
                self.toast = Some(format!(
                    "'{}' is checked out in another worktree at {} — checkout blocked",
                    b.name,
                    wt.display()
                ));
                return;
            }
        }
        if !self.status.files.is_empty() {
            self.toast = Some(format!(
                "checkout '{}' blocked: working tree has changes (commit / stash / discard first)",
                b.name
            ));
            return;
        }
        let name = b.name.clone();
        let root = self.repo.root.clone();
        self.run_op_async(
            &format!("checkout {name}"),
            Box::pin(async move { git::ops::checkout(&root, &name).await }),
        );
    }

    /// Dispatch a Branches-pane action — same effect as the matching keyboard
    /// shortcut. Used by toolbar button clicks.
    pub fn run_branch_action(&mut self, action: BranchAction) {
        // Make sure the user's mental model matches: clicking a button focuses
        // the Branches pane.
        self.active_pane = Pane::Branches;
        match action {
            BranchAction::Checkout => self.checkout_selected_branch(),
            BranchAction::NewBranch => self.start_new_branch(),
            BranchAction::Push => self.spawn_push(),
            BranchAction::Pull => self.run_op_async(
                "pull --ff-only",
                Box::pin({
                    let r = self.repo.root.clone();
                    async move { git::ops::pull(&r).await }
                }),
            ),
            BranchAction::Fetch => self.run_op_async(
                "fetch --all",
                Box::pin({
                    let r = self.repo.root.clone();
                    async move { git::ops::fetch_all(&r).await }
                }),
            ),
            BranchAction::Merge => self.confirm_merge(),
            BranchAction::Delete => self.confirm_delete(false),
        }
    }

    fn scroll_overlay(&mut self, delta: isize) {
        let max = self
            .overlay
            .as_ref()
            .map(|o| o.line_count() as isize - 1)
            .unwrap_or(0)
            .max(0);
        let next = (self.overlay_scroll as isize + delta).clamp(0, max);
        self.overlay_scroll = next as u16;
    }

    async fn open_details(&mut self) {
        let (title, fetch): (String, BoxedFetch) = match self.active_pane {
            Pane::Changes => {
                let Some(idx) = self.changes_state.selected() else {
                    return;
                };
                let Some(file) = self.status.files.get(idx) else {
                    return;
                };
                let path = file.path.clone();
                let from = file.from.clone();
                let kind = file.kind;
                let staged = file.staged.is_some();
                let title = match (&from, kind) {
                    (Some(f), _) => format!("{f} → {path}"),
                    _ => path.clone(),
                };
                let root = self.repo.root.clone();
                (
                    title,
                    Box::pin(async move {
                        match kind {
                            ChangeKind::Untracked => git::diff::untracked(&root, &path).await,
                            _ => git::diff::file(&root, &path, staged).await,
                        }
                    }),
                )
            }
            Pane::Graph => {
                let Some(idx) = self.graph_state.selected() else {
                    return;
                };
                let Some(commit) = self.commits.get(idx) else {
                    return;
                };
                let title = format!("{} {}", &commit.short_hash, &commit.subject);
                let sha = commit.hash.clone();
                let root = self.graph_root.clone();
                (
                    title,
                    Box::pin(async move { git::diff::show(&root, &sha).await }),
                )
            }
            Pane::Branches => return,
        };

        self.overlay = Some(DetailsContent::Loading {
            title: title.clone(),
        });
        self.overlay_scroll = 0;

        let tx = self.events_tx.clone();
        tokio::spawn(async move {
            let content = match fetch.await {
                Ok(body) => DetailsContent::Body { title, body },
                Err(e) => DetailsContent::Error {
                    title,
                    message: format!("{e}"),
                },
            };
            let _ = tx.send(AppEvent::OverlayLoaded(content)).await;
        });
    }

    fn move_selection(&mut self, delta: isize) {
        let (state, len) = self.active_state_and_len();
        if len == 0 {
            return;
        }
        let cur = state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, len as isize - 1) as usize;
        state.select(Some(next));
        self.handle_selection_changed();
    }

    fn move_selection_to(&mut self, target: isize) {
        let (state, len) = self.active_state_and_len();
        if len == 0 {
            return;
        }
        let next = target.clamp(0, len as isize - 1) as usize;
        state.select(Some(next));
        self.handle_selection_changed();
    }

    fn handle_selection_changed(&mut self) {
        if self.active_pane == Pane::Branches {
            self.refresh_graph_for_selected_branch();
        }
        self.update_preview();
    }

    fn active_state_and_len(&mut self) -> (&mut ListState, usize) {
        match self.active_pane {
            Pane::Branches => (&mut self.branches_state, self.branches.len()),
            Pane::Changes => (&mut self.changes_state, self.status.files.len()),
            Pane::Graph => (&mut self.graph_state, self.commits.len()),
        }
    }

    /// Compute (target_id, title, fetch_future) for the current pane+selection.
    /// Returns None when there's nothing to preview (e.g. on Branches pane).
    fn current_preview_request(&self) -> Option<(String, String, BoxedFetch)> {
        match self.active_pane {
            Pane::Graph => {
                let idx = self.graph_state.selected()?;
                let commit = self.commits.get(idx)?;
                let target = format!("sha:{}:{}", self.graph_root.display(), &commit.hash);
                let title = format!("{} {}", &commit.short_hash, &commit.subject);
                let sha = commit.hash.clone();
                let root = self.graph_root.clone();
                Some((
                    target,
                    title,
                    Box::pin(async move { git::diff::show(&root, &sha).await }),
                ))
            }
            Pane::Changes => {
                let idx = self.changes_state.selected()?;
                let file = self.status.files.get(idx)?;
                let staged = file.staged.is_some() && file.unstaged.is_none();
                let target = format!("file:{}:{}", staged as u8, &file.path);
                let title = match (&file.from, file.kind) {
                    (Some(f), _) => format!("{f} → {}", &file.path),
                    _ => file.path.clone(),
                };
                let path = file.path.clone();
                let kind = file.kind;
                let root = self.repo.root.clone();
                Some((
                    target,
                    title,
                    Box::pin(async move {
                        match kind {
                            ChangeKind::Untracked => git::diff::untracked(&root, &path).await,
                            _ => git::diff::file(&root, &path, staged).await,
                        }
                    }),
                ))
            }
            Pane::Branches => None,
        }
    }

    /// Inspect the current pane+selection and update the inline preview if the
    /// target changed. Cheap when target is the same (no fetch spawned).
    fn update_preview(&mut self) {
        let request = self.current_preview_request();
        match request {
            None => {
                // No valid target — clear the preview.
                self.preview = None;
                self.preview_target = None;
                self.preview_scroll = 0;
            }
            Some((target, title, fetch)) => {
                if self.preview_target.as_deref() == Some(target.as_str()) {
                    return; // already showing this
                }
                self.preview_target = Some(target.clone());
                self.preview = Some(DetailsContent::Loading {
                    title: title.clone(),
                });
                self.preview_scroll = 0;

                let tx = self.events_tx.clone();
                tokio::spawn(async move {
                    let content = match fetch.await {
                        Ok(body) => DetailsContent::Body { title, body },
                        Err(e) => DetailsContent::Error {
                            title,
                            message: format!("{e}"),
                        },
                    };
                    let _ = tx.send(AppEvent::PreviewLoaded(target, content)).await;
                });
            }
        }
    }

    fn refresh_graph_for_selected_branch(&mut self) {
        if self.sync_graph_source_to_branch_selection() {
            self.spawn_graph_refresh();
        }
    }

    fn sync_graph_source_to_branch_selection(&mut self) -> bool {
        let (branch, upstream, root) = self.selected_graph_source();
        if self.graph_branch == branch && self.graph_upstream == upstream && self.graph_root == root
        {
            return false;
        }

        self.graph_branch = branch;
        self.graph_upstream = upstream;
        self.graph_root = root;
        self.commits.clear();
        self.ahead_shas.clear();
        self.behind_shas.clear();
        self.graph_state.select(Some(0));
        self.preview = None;
        self.preview_target = None;
        self.preview_scroll = 0;
        true
    }

    fn selected_graph_source(&self) -> (Option<String>, Option<String>, PathBuf) {
        if let Some(branch) = self.selected_branch() {
            return self.graph_source_for_branch(branch);
        }

        match &self.head {
            HeadRef::Branch(name) => {
                if let Some(branch) = self.branches.iter().find(|b| b.name == *name) {
                    self.graph_source_for_branch(branch)
                } else {
                    (
                        Some(name.clone()),
                        None,
                        self.worktree_root_for_branch(name)
                            .unwrap_or_else(|| self.repo.root.clone()),
                    )
                }
            }
            HeadRef::Detached(_) | HeadRef::Unborn => (None, None, self.repo.root.clone()),
        }
    }

    fn selected_branch(&self) -> Option<&Branch> {
        self.branches_state
            .selected()
            .and_then(|idx| self.branches.get(idx))
    }

    fn graph_source_for_branch(
        &self,
        branch: &Branch,
    ) -> (Option<String>, Option<String>, PathBuf) {
        (
            Some(branch.name.clone()),
            branch.upstream.clone(),
            self.worktree_root_for_branch(&branch.name)
                .unwrap_or_else(|| self.repo.root.clone()),
        )
    }

    fn worktree_root_for_branch(&self, branch: &str) -> Option<PathBuf> {
        self.worktrees.get(branch).cloned()
    }

    fn spawn_graph_refresh(&mut self) {
        self.graph_request_id = self.graph_request_id.wrapping_add(1);
        let request_id = self.graph_request_id;
        let tx = self.events_tx.clone();
        let root = self.graph_root.clone();
        let branch = self.graph_branch.clone();
        let upstream = self.graph_upstream.clone();
        let label = branch
            .as_deref()
            .map(|b| format!("log {b}"))
            .unwrap_or_else(|| "log".to_string());

        tokio::spawn(async move {
            let result = match branch.as_deref() {
                Some(name) => git::log::fetch_branch(&root, name, upstream.as_deref(), 500).await,
                None => git::log::fetch(&root, 500).await,
            };
            match result {
                Ok(commits) => {
                    let _ = tx
                        .send(AppEvent::CommitsLoaded {
                            request_id,
                            commits,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::LoadFailed(format!("{label}: {e}"))).await;
                }
            }
        });

        let tx = self.events_tx.clone();
        let root = self.graph_root.clone();
        let branch = self.graph_branch.clone();
        tokio::spawn(async move {
            let (ahead, behind) = match branch.as_deref() {
                Some(name) => {
                    let ahead = git::log::ahead_of_branch(&root, name).await;
                    let behind = git::log::behind_branch(&root, name).await;
                    (ahead, behind)
                }
                None => {
                    let ahead = git::log::ahead_of_upstream(&root).await;
                    let behind = git::log::behind_upstream(&root).await;
                    (ahead, behind)
                }
            };
            let _ = tx
                .send(AppEvent::DivergenceLoaded {
                    request_id,
                    ahead,
                    behind,
                })
                .await;
        });
    }

    fn spawn_pr_refresh(&self) {
        let tx = self.events_tx.clone();
        let root = self.repo.root.clone();
        tokio::spawn(async move {
            match gh::fetch_prs(&root).await {
                Ok(map) => {
                    let _ = tx.send(AppEvent::PrsLoaded(map)).await;
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::LoadFailed(format!("gh prs: {e}"))).await;
                }
            }
        });
    }

    fn spawn_refresh(&mut self) {
        let tx = self.events_tx.clone();
        let root = self.repo.root.clone();
        tokio::spawn(async move {
            match git::branches::list(&root).await {
                Ok(b) => {
                    let _ = tx.send(AppEvent::BranchesLoaded(b)).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AppEvent::LoadFailed(format!("branches: {e}")))
                        .await;
                }
            }
        });

        self.sync_graph_source_to_branch_selection();
        self.spawn_graph_refresh();

        let tx = self.events_tx.clone();
        let root = self.repo.root.clone();
        tokio::spawn(async move {
            match git::status::fetch(&root).await {
                Ok(s) => {
                    let _ = tx.send(AppEvent::StatusLoaded(s)).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AppEvent::LoadFailed(format!("status: {e}")))
                        .await;
                }
            }
        });

        let tx = self.events_tx.clone();
        let repo = self.repo.clone();
        tokio::spawn(async move {
            if let Ok(h) = repo.head().await {
                let _ = tx.send(AppEvent::HeadLoaded(h)).await;
            }
        });

        // Worktree map — for the "checked out elsewhere" indicator.
        let tx = self.events_tx.clone();
        let root = self.repo.root.clone();
        tokio::spawn(async move {
            if let Ok(map) = git::worktree::list(&root).await {
                let _ = tx.send(AppEvent::WorktreesLoaded(map)).await;
            }
        });

        // PR refresh: throttled to once per 60s.
        if self.gh == Availability::Ready
            && self.prs_last_refresh.elapsed() >= Duration::from_secs(60)
        {
            self.spawn_pr_refresh();
        }
    }
}

/// True when the point (x, y) lies inside `rect` (inclusive of borders so a
/// click on a pane border still focuses that pane).
fn hit(rect: Rect, x: u16, y: u16) -> bool {
    rect.width > 0
        && rect.height > 0
        && x >= rect.x
        && x < rect.x + rect.width
        && y >= rect.y
        && y < rect.y + rect.height
}

/// `git@github.com:owner/repo.git` or `https://github.com/owner/repo(.git)?` → web URL.
fn github_web_url(remote: &str) -> Option<String> {
    let r = remote.trim().trim_end_matches(".git");
    if let Some(rest) = r.strip_prefix("git@github.com:") {
        return Some(format!("https://github.com/{rest}"));
    }
    if r.starts_with("https://github.com/") || r.starts_with("http://github.com/") {
        return Some(r.to_string());
    }
    if let Some(rest) = r.strip_prefix("ssh://git@github.com/") {
        return Some(format!("https://github.com/{rest}"));
    }
    None
}

/// Cap a staged diff before sending to Ollama. Most local models choke on
/// 100k-token inputs and the commit subject is determined by the high-level
/// file/hunk structure anyway, so chopping at ~8k lines / 32 KB is a sane
/// budget. The boolean signals whether truncation actually happened so the
/// prompt can disclose that.
const MAX_DIFF_LINES: usize = 8000;
const MAX_DIFF_BYTES: usize = 32 * 1024;

fn truncate_diff(diff: &str) -> (String, bool) {
    let mut out = String::new();
    let mut truncated = false;
    for (i, line) in diff.lines().enumerate() {
        if i >= MAX_DIFF_LINES || out.len() + line.len() + 1 > MAX_DIFF_BYTES {
            truncated = true;
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    (out, truncated)
}

/// Coerce the raw streamed model output into a single commit subject line:
/// take the first non-empty line, strip stray quotes some models add, and cap
/// the length defensively.
fn clean_subject(raw: &str) -> String {
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
        .trim()
        .to_string();
    if line.chars().count() > 200 {
        line.chars().take(200).collect()
    } else {
        line
    }
}

fn clamp_selection(state: &mut ListState, len: usize) {
    if len == 0 {
        state.select(None);
        return;
    }
    let cur = state.selected().unwrap_or(0).min(len - 1);
    state.select(Some(cur));
}

/// Helper for `main.rs` to glue everything together.
pub async fn launch(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    repo: Repo,
) -> Result<()> {
    let (tx, rx) = spawn_event_loop(64, Duration::from_secs(2));
    let mut app = App::new(repo.clone(), tx.clone()).await?;

    // Detect gh in the background so startup is instant.
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let avail = gh::detect().await;
            let _ = tx.send(AppEvent::GhAvailability(avail)).await;
        });
    }

    // Detect a local Ollama instance — same async-on-startup pattern as `gh`.
    // Uses the base URL from the (possibly just-loaded) user config.
    {
        let tx = tx.clone();
        let base_url = app.config.ollama.base_url.clone();
        tokio::spawn(async move {
            let avail = ollama::detect(&base_url).await;
            let _ = tx.send(AppEvent::OllamaAvailability(avail)).await;
        });
    }

    // .git/ watcher — instant refresh when other terminals mutate the repo.
    let _ = crate::watcher::spawn(&repo.root, tx);

    app.run(terminal, rx).await
}
