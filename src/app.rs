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
    graph,
    ollama::{self, Availability as OllamaAvailability},
    ui::{
        self,
        panes::{
            commit_input::InputState,
            confirm::{ConfirmAction, ConfirmDialog},
            details::DetailsContent,
            generation::{GenerationDialog, GenerationPhase},
            info::InfoDialog,
            model_picker::ModelPickerState,
            settings::{FieldEditor, SettingsRow, SettingsState},
        },
    },
};

type BoxedFetch = Pin<Box<dyn Future<Output = Result<String>> + Send>>;
type BoxedOp = Pin<Box<dyn Future<Output = Result<String>> + Send>>;

/// Hard cap on commits loaded into the Graph pane. Bounded so the worst-case
/// (200k-commit monorepo) doesn't OOM, but high enough to scroll real history.
/// When this limit is hit, `App::graph_truncated` is set and the title shows
/// `5000+` instead of an exact count.
pub const MAX_GRAPH_COMMITS: usize = 5000;

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
    /// Just the row area each pane's `List` widget occupies — i.e. the pane's
    /// inner rect minus the toolbar (and the inline commit-message box, when
    /// open in Changes). Click hit-testing uses this so a click maps to the
    /// row directly under the cursor regardless of how tall the toolbar
    /// wrapped to or whether the commit box is visible.
    pub branches_list: Rect,
    pub changes_list: Rect,
    pub graph_list: Rect,
    /// On-screen rect of the `ollama: ✓ <model>` segment in the status bar.
    /// Click here = open model picker. Empty when not rendered.
    pub status_ollama: Option<Rect>,
    /// On-screen rect of the `[ ⚙ settings ]` chip in the status bar.
    /// Click here = open settings modal.
    pub status_settings: Option<Rect>,
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

/// Actions exposed as clickable buttons in the Changes pane toolbar.
/// Same dispatch as the keyboard shortcuts (`c` / `C` / `^G` / `a` / `A` / `r`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeAction {
    Commit,
    CommitAndPush,
    AiMessage,
    StageAll,
    UnstageAll,
    Uncommit,
    Refresh,
    /// Preview every uncommitted change in one combined diff. The user can
    /// still click an individual file row afterwards to drop back into the
    /// per-file view.
    ViewAll,
}

/// Top-level application state.
pub struct App {
    pub repo: Repo,
    pub events_tx: mpsc::Sender<AppEvent>,

    pub head: HeadRef,
    pub branches: Vec<Branch>,
    pub commits: Vec<Commit>,
    /// True when `commits` was capped at `MAX_GRAPH_COMMITS` — i.e. there's
    /// older history we didn't load. Used to disclose `5000+` in the title.
    pub graph_truncated: bool,
    /// Cached lane layout for `commits`. Recomputed only when commits change
    /// (see invalidations at `CommitsLoaded` and `sync_graph_source_to_branch_selection`).
    /// Avoids paying O(N) lane allocation on every render frame.
    pub graph_layout: Option<Vec<graph::Row>>,
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
    /// True when the preview was set explicitly (e.g. "view all changes"
    /// button) and shouldn't be overwritten by background status refreshes.
    /// Cleared the moment the user navigates (selection move, pane switch,
    /// row click) — i.e. any user-driven `update_preview` call.
    pub preview_pinned: bool,

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
    /// Active settings modal, if open.
    pub settings: Option<SettingsState>,
    /// Active commit-message generation modal — present while streaming, when
    /// the result is ready for review, and when an error needs to be shown.
    pub generation: Option<GenerationDialog>,
    /// Active info / error popup. Used for any message the user shouldn't
    /// risk missing (op failures, blocked deletes, etc.) — replaces the
    /// bottom toast for diagnostic-grade output.
    pub info_dialog: Option<InfoDialog>,
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
    /// Same idea for the Changes-pane toolbar.
    pub change_button_rects: Vec<(ChangeAction, Rect)>,
    /// Per-row `+`/`−` toggle buttons in the Changes pane: tuple is
    /// `(file_index_in_status, rect)`. Empty when toolbar didn't render.
    pub change_file_button_rects: Vec<(usize, Rect)>,
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
            graph_truncated: false,
            graph_layout: None,
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
            preview_pinned: false,
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
            settings: None,
            generation: None,
            info_dialog: None,
            gen_task: None,
            layout_overrides: LayoutOverrides::default(),
            active_drag: ResizeDrag::None,
            branch_button_rects: Vec::new(),
            change_button_rects: Vec::new(),
            change_file_button_rects: Vec::new(),
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
            AppEvent::Mouse(m) => self.handle_mouse(m).await,
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
                    self.graph_truncated = commits.len() >= MAX_GRAPH_COMMITS;
                    self.commits = commits;
                    self.graph_layout = None;
                    self.refresh_preview_after_reload();
                }
            }
            AppEvent::StatusLoaded(s) => {
                clamp_selection(&mut self.changes_state, s.files.len());
                self.status = s;
                self.refresh_preview_after_reload();
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
            AppEvent::OpFailed { label, error } => {
                let title = format!("{label} failed");
                let hint = suggest_op_hint(&label, &error);
                let mut dialog = InfoDialog::error(title, error);
                if let Some(h) = hint {
                    dialog = dialog.with_hint(h);
                }
                self.info_dialog = Some(dialog);
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
                // Live-refresh the picker if it's open. Whatever the new
                // availability is, we hand the fresh list (or empty) to the
                // picker so the user sees the up-to-date state without
                // closing/reopening.
                if let Some(picker) = self.model_picker.as_mut() {
                    let fresh: Vec<String> = match &self.ollama {
                        OllamaAvailability::Ready { models } => models.clone(),
                        _ => Vec::new(),
                    };
                    let current = self.config.ollama.model.as_deref();
                    picker.replace_models(fresh, current);
                }
            }
            AppEvent::OllamaToken(tok) => {
                // Route into both the modal preview and the inline buf — the
                // inline buf is what gets committed on accept, the modal is
                // what the user reads while it streams.
                if let Some(g) = self.generation.as_mut() {
                    g.append_partial(&tok);
                }
                if self.input.generating {
                    self.input.append(&tok);
                }
            }
            AppEvent::OllamaDone(full) => {
                let cleaned = clean_message(&full);
                if cleaned.is_empty() {
                    // Empty cleaned-subject = the model returned nothing
                    // usable. Most common reasons:
                    //   - user picked an embedding model (no `response` field)
                    //   - model emitted only a `<think>` block with no answer
                    //   - model genuinely returned an empty completion
                    // Showing an empty "ready" modal would hide all of these.
                    // Fail loudly via the same modal so retry / model-pick are
                    // one keystroke away.
                    let preview: String = full.trim().chars().take(160).collect();
                    let model_name = self
                        .generation
                        .as_ref()
                        .map(|g| g.model.clone())
                        .unwrap_or_else(|| "model".to_string());
                    let msg = if preview.is_empty() {
                        format!(
                            "{model_name} returned no text — likely an embedding model that can't generate prose. \
                             Press r to retry or Esc to dismiss; the model stays as-is."
                        )
                    } else {
                        format!(
                            "{model_name} returned only reasoning content — no usable subject line was extracted. \
                             Press r to retry. Raw start: {preview}…"
                        )
                    };
                    if self.input.generating || self.input_mode == InputMode::Commit {
                        self.input.clear();
                        self.input_mode = InputMode::Normal;
                    }
                    if let Some(g) = self.generation.as_mut() {
                        g.finish_error(msg);
                    } else {
                        self.toast = Some(msg);
                    }
                    self.gen_task = None;
                } else {
                    if self.input.generating {
                        self.input.generating = false;
                        self.input.buf = cleaned.clone();
                        // Park the cursor at the end of the subject line, not
                        // the end of the body. The inline input bar is one
                        // row, so this is where the user can actually see and
                        // edit; the body is committed but isn't part of the
                        // inline preview.
                        self.input.cursor = cleaned.find('\n').unwrap_or(cleaned.len());
                    }
                    if let Some(g) = self.generation.as_mut() {
                        g.finish_done(cleaned);
                    }
                    self.gen_task = None;
                }
            }
            AppEvent::OllamaError(msg) => {
                // Surface the error in the generation modal (so it's loud,
                // not a silent toast). Keep the inline input cleared so the
                // user doesn't accidentally commit a half-streamed message.
                if self.input.generating || self.input_mode == InputMode::Commit {
                    self.input.clear();
                    self.input_mode = InputMode::Normal;
                }
                if let Some(g) = self.generation.as_mut() {
                    g.finish_error(msg.clone());
                } else {
                    // Fallback: no modal up (shouldn't happen post-refactor),
                    // toast as a last resort so the user still sees it.
                    self.toast = Some(msg);
                }
                self.gen_task = None;
            }
        }

        // Spinner animation: any event that fires while a generation modal is
        // streaming bumps the frame so the UI feels alive even if tokens are
        // sparse. Cheap modular increment.
        if let Some(g) = self.generation.as_mut() {
            if g.is_streaming() {
                g.spinner = g.spinner.wrapping_add(1);
            }
        }
    }

    async fn handle_key(&mut self, key: KeyEvent) {
        // global Ctrl-C quits unconditionally
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        // Generation modal owns the screen above ALL other modals when active.
        // While streaming, only Esc cancels; once Done/Error, the user reviews.
        if self.generation.is_some() {
            self.handle_generation_key(key).await;
            return;
        }

        // Info / error popup is dismiss-only — Esc or Enter closes it.
        // Anything else is ignored so a stray keystroke doesn't trigger a
        // hidden action behind the popup.
        if self.info_dialog.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    self.info_dialog = None;
                }
                _ => {}
            }
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
                || self.settings.is_some()
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

        // Settings modal has top-level priority once open.
        if self.settings.is_some() {
            self.handle_settings_key(key);
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
            KeyCode::PageDown => {
                let step = self.active_viewport_height().max(1);
                self.move_selection(step as isize);
            }
            KeyCode::PageUp => {
                let step = self.active_viewport_height().max(1);
                self.move_selection(-(step as isize));
            }
            KeyCode::Char('d') | KeyCode::Char('D')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let step = (self.active_viewport_height() / 2).max(1);
                self.move_selection(step as isize);
            }
            KeyCode::Char('u') | KeyCode::Char('U')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let step = (self.active_viewport_height() / 2).max(1);
                self.move_selection(-(step as isize));
            }
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
            KeyCode::Char('U') if self.active_pane == Pane::Changes => {
                self.start_uncommit_flow().await;
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

            // Open the Settings modal — ',' is a common "preferences" mnemonic
            // (Cmd-, on macOS apps). The model picker is reachable from there
            // (and from the `ollama: ✓ …` chip in the status bar). A bare `M`
            // shortcut used to open the picker directly, but `M` collides with
            // vim's "middle of screen" muscle memory and was opening it during
            // navigation; that path was removed.
            KeyCode::Char(',') => self.open_settings(),
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
    async fn handle_mouse(&mut self, ev: MouseEvent) {
        let pos = (ev.column, ev.row);

        // Generation modal blocks ALL mouse input — the user can't accidentally
        // navigate while the AI is busy. They have to use the keyboard
        // (Esc / Enter / r) to advance.
        if self.generation.is_some() {
            return;
        }

        // Info / error popup also blocks mouse: clicking anywhere just
        // dismisses the popup so the user can keep working.
        if self.info_dialog.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = ev.kind {
                self.info_dialog = None;
            }
            return;
        }

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

        // Settings + model-picker modals consume left-clicks themselves so
        // rows / list items become clickable. Other events (scroll, drag) are
        // ignored while a modal owns the screen.
        if self.settings.is_some() {
            if let MouseEventKind::Down(MouseButton::Left) = ev.kind {
                self.handle_settings_click(pos);
            }
            return;
        }
        if self.model_picker.is_some() {
            // Picker doesn't expose row rects yet; ignore clicks.
            return;
        }

        // Block the mouse only while the AI is actively streaming tokens —
        // the screen would race the live text otherwise. Once the stream
        // ends (success, error, or cancel) we want the user to be able to
        // click ✓ commit, scroll the diff, drag a splitter, etc., without
        // having to first dismiss commit-input mode. Pane-content clicks
        // are still safe here: they change selection but never the input
        // buffer.
        if self.input.generating {
            return;
        }

        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => self.handle_mouse_click(pos).await,
            MouseEventKind::Drag(MouseButton::Left) => self.handle_mouse_drag(pos),
            MouseEventKind::Up(MouseButton::Left) => {
                self.active_drag = ResizeDrag::None;
            }
            MouseEventKind::ScrollUp => self.handle_mouse_scroll(pos, -3),
            MouseEventKind::ScrollDown => self.handle_mouse_scroll(pos, 3),
            _ => {}
        }
    }

    async fn handle_mouse_click(&mut self, (x, y): (u16, u16)) {
        // First: status-bar chips. They sit on the bottom row, outside any
        // pane, so we hit-test them before splitters / pane content.
        if let Some(rect) = self.last_rects.status_ollama {
            if hit(rect, x, y) {
                self.open_model_picker();
                return;
            }
        }
        if let Some(rect) = self.last_rects.status_settings {
            if hit(rect, x, y) {
                self.open_settings();
                return;
            }
        }

        // Second: did they grab a splitter? If so, start a drag and don't also
        // select an item.
        if let Some(drag) = self.detect_splitter(x, y) {
            self.active_drag = drag;
            return;
        }

        // Third: did they click a Branches-pane toolbar button?
        if let Some(action) = self.detect_branch_button(x, y) {
            self.run_branch_action(action);
            return;
        }

        // Changes-pane toolbar buttons.
        if let Some(action) = self.detect_change_button(x, y) {
            self.active_pane = Pane::Changes;
            self.run_change_action(action).await;
            return;
        }

        // Per-row +/− stage toggle buttons in the Changes pane.
        if let Some(idx) = self.detect_change_file_button(x, y) {
            self.active_pane = Pane::Changes;
            self.changes_state.select(Some(idx));
            self.update_preview();
            self.toggle_stage_at(idx).await;
            return;
        }

        let rects = self.last_rects.clone();
        if hit(rects.branches, x, y) {
            self.active_pane = Pane::Branches;
            self.click_select_in_pane(rects.branches_list, y, Pane::Branches);
            self.refresh_graph_for_selected_branch();
            self.update_preview();
        } else if hit(rects.changes, x, y) {
            self.active_pane = Pane::Changes;
            self.click_select_in_pane(rects.changes_list, y, Pane::Changes);
            self.update_preview();
        } else if hit(rects.graph, x, y) {
            self.active_pane = Pane::Graph;
            self.click_select_in_pane(rects.graph_list, y, Pane::Graph);
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

    fn detect_change_button(&self, x: u16, y: u16) -> Option<ChangeAction> {
        self.change_button_rects
            .iter()
            .find(|(_, r)| hit(*r, x, y))
            .map(|(a, _)| *a)
    }

    fn detect_change_file_button(&self, x: u16, y: u16) -> Option<usize> {
        self.change_file_button_rects
            .iter()
            .find(|(_, r)| hit(*r, x, y))
            .map(|(idx, _)| *idx)
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

    /// `list_rect` is the exact area the pane's `List` widget paints into
    /// (after subtracting borders, toolbar, and the inline commit box). A click
    /// inside that rect maps directly to a row — no row-offset guessing.
    fn click_select_in_pane(&mut self, list_rect: Rect, y: u16, pane: Pane) {
        if list_rect.height == 0 || y < list_rect.y || y >= list_rect.y + list_rect.height {
            return;
        }
        let row_in_list = (y - list_rect.y) as usize;
        let (state, len) = match pane {
            Pane::Branches => (&mut self.branches_state, self.branches.len()),
            Pane::Changes => (&mut self.changes_state, self.status.files.len()),
            Pane::Graph => (&mut self.graph_state, self.commits.len()),
        };
        let target = state.offset() + row_in_list;
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
        if let Some(idx) = self.changes_state.selected() {
            self.toggle_stage_at(idx).await;
        }
    }

    /// Toggle stage state for `idx` regardless of current selection. Used by
    /// the per-row `+`/`−` button click in the Changes pane.
    pub async fn toggle_stage_at(&mut self, idx: usize) {
        let Some(file) = self.status.files.get(idx) else {
            return;
        };
        let path = file.path.clone();
        let staged_already = is_fully_staged(file);
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

    /// Dispatch a clicked Changes-pane toolbar button. Same effect as the
    /// keyboard shortcuts that already exist.
    pub async fn run_change_action(&mut self, action: ChangeAction) {
        match action {
            ChangeAction::StageAll => self.run_op_async(
                "stage all",
                Box::pin({
                    let r = self.repo.root.clone();
                    async move { git::ops::stage_all(&r).await }
                }),
            ),
            ChangeAction::UnstageAll => self.run_op_async(
                "unstage all",
                Box::pin({
                    let r = self.repo.root.clone();
                    async move { git::ops::unstage_all(&r).await }
                }),
            ),
            ChangeAction::Commit => {
                // If we're already in commit mode with a ready message
                // (e.g. just generated by AI, or manually typed), treat
                // the click as "submit" instead of wiping the buffer and
                // opening an empty input.
                if self.input_mode == InputMode::Commit
                    && !self.input.generating
                    && !self.input.buf.trim().is_empty()
                {
                    self.do_commit().await;
                } else if self.input_mode != InputMode::Commit
                    && self.status.files.is_empty()
                {
                    // No working-tree changes and no in-flight input — opening
                    // an empty commit box would just frustrate the user.
                    self.toast = Some("nothing to commit".to_string());
                } else {
                    self.input_mode = InputMode::Commit;
                    self.input.clear();
                }
            }
            ChangeAction::CommitAndPush => {
                if self.input_mode == InputMode::Commit
                    && !self.input.generating
                    && !self.input.buf.trim().is_empty()
                {
                    self.input.push_after = true;
                    self.do_commit().await;
                } else if self.input_mode != InputMode::Commit
                    && self.status.files.is_empty()
                {
                    // Nothing to commit — fall through to a plain push of the
                    // current branch. Same code path as the Branches-pane Push
                    // button, so this button stays useful after the user has
                    // already committed locally.
                    self.spawn_push();
                } else {
                    self.input_mode = InputMode::Commit;
                    self.input.clear();
                    self.input.push_after = true;
                }
            }
            ChangeAction::AiMessage => {
                // Drop into commit-input mode (so the streamed message has
                // somewhere to go) and kick off generation. Same effect the
                // user would get from `c` followed by `Ctrl-G`.
                if !self.input.generating {
                    self.start_generation().await;
                }
            }
            ChangeAction::Uncommit => self.start_uncommit_flow().await,
            ChangeAction::Refresh => self.spawn_refresh(),
            ChangeAction::ViewAll => self.show_all_changes_preview(),
        }
    }

    /// Open the uncommit confirmation dialog if it's safe — i.e. HEAD is on a
    /// branch, has a parent commit, and the commit hasn't been pushed yet
    /// (otherwise rewriting HEAD locally would create divergence). Each guard
    /// surfaces a specific toast so the user knows why it was blocked.
    async fn start_uncommit_flow(&mut self) {
        let branch = match &self.head {
            HeadRef::Branch(b) => b.clone(),
            HeadRef::Detached(_) => {
                self.toast = Some("HEAD is detached — uncommit blocked".to_string());
                return;
            }
            HeadRef::Unborn => {
                self.toast = Some("repo has no commits yet".to_string());
                return;
            }
        };
        // If the branch has an upstream and is not ahead of it, the HEAD
        // commit was already pushed — uncommitting would create divergence.
        if let Some(b) = self.branches.iter().find(|b| b.name == branch) {
            if b.upstream.is_some() && b.ahead == 0 {
                self.toast = Some(format!(
                    "'{branch}' is in sync with upstream — uncommit would force-push later, blocked"
                ));
                return;
            }
        }
        let summary = match git::ops::head_summary(&self.repo.root).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                self.toast = Some("no HEAD commit to uncommit".to_string());
                return;
            }
            Err(e) => {
                self.toast = Some(format!("uncommit preflight: {e}"));
                return;
            }
        };
        if !summary.has_parent {
            self.toast = Some("HEAD is the root commit — cannot uncommit".to_string());
            return;
        }
        self.confirm = Some(ConfirmDialog {
            title: "uncommit".to_string(),
            message: format!(
                "Uncommit '{}' ({})? Changes return to the staging area — reversible via reflog.",
                summary.subject, summary.short
            ),
            action: ConfirmAction::Uncommit,
        });
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
            self.info_dialog = Some(
                InfoDialog::error(
                    "Cannot delete current branch",
                    format!(
                        "'{}' is checked out in this worktree. Switch to another branch first \
                         (Branches pane → Enter on the target), then retry the delete.",
                        b.name
                    ),
                ),
            );
            return;
        }
        // If the branch is checked out in another worktree, we have to
        // `git worktree remove` it before `git branch -d` will succeed.
        // Bake that into the confirm flow so a single Yes does both.
        let worktree = self.worktrees.get(&b.name).cloned();
        let title = match (force, worktree.is_some()) {
            (true, _) => "Force-delete branch",
            (false, true) => "Delete branch and worktree",
            (false, false) => "Delete branch",
        };
        let message = match (force, &worktree) {
            (true, Some(p)) => format!(
                "Force-delete branch '{}' (loses unmerged commits) AND remove the worktree at {}? \
                 If the worktree has uncommitted changes they will be discarded.",
                b.name,
                p.display()
            ),
            (true, None) => format!(
                "Force-delete branch '{}' (loses unmerged commits)?",
                b.name
            ),
            (false, Some(p)) => format!(
                "Delete branch '{}' AND remove the worktree at {}? If the worktree has \
                 uncommitted changes they will be discarded.",
                b.name,
                p.display()
            ),
            (false, None) => format!("Delete branch '{}'?", b.name),
        };
        self.confirm = Some(ConfirmDialog {
            title: title.to_string(),
            message,
            action: ConfirmAction::DeleteBranch {
                name: b.name.clone(),
                force,
                worktree,
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
            ConfirmAction::DeleteBranch {
                name,
                force,
                worktree,
            } => {
                let label = format!("delete {name}");
                let op: BoxedOp = Box::pin(async move {
                    // Worktree first — `git branch -d/-D` refuses while a
                    // branch is checked out anywhere. Try clean remove, then
                    // force-fallback (the user already opted into destruction
                    // by typing `y` on the confirm dialog).
                    if let Some(path) = worktree {
                        if git::worktree::remove(&root, &path).await.is_err() {
                            git::worktree::remove_force(&root, &path).await?;
                        }
                    }
                    if force {
                        git::ops::force_delete_branch(&root, &name).await
                    } else {
                        git::ops::delete_branch(&root, &name).await
                    }
                });
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
            ConfirmAction::Uncommit => {
                let op: BoxedOp = Box::pin(async move { git::ops::uncommit_soft(&root).await });
                self.run_op_async("uncommit", op);
            }
        }
    }

    /// Open the Ollama model picker modal. Always kicks off a fresh
    /// `/api/tags` probe so the listed models reflect the live state of the
    /// daemon (handy when the user just `ollama pull`-ed a new model in
    /// another terminal). The picker opens immediately with whatever was
    /// cached and swaps to the fresh list when the probe lands.
    fn open_model_picker(&mut self) {
        let cached = match &self.ollama {
            OllamaAvailability::Ready { models } => models.clone(),
            // Don't gate the picker behind a stale "not running" status —
            // the user may have just started Ollama, and the fresh probe
            // we're about to fire will tell us if that worked. Open the
            // modal in refreshing state and let the result populate it.
            _ => Vec::new(),
        };
        let current = self.config.ollama.model.as_deref();
        let mut picker = ModelPickerState::new(cached, current);
        picker.refreshing = true;
        self.model_picker = Some(picker);
        self.spawn_ollama_probe();
    }

    /// Fire a one-shot `/api/tags` probe. Result lands on the event loop as
    /// `AppEvent::OllamaAvailability`; the handler updates `self.ollama` and
    /// (if open) refreshes the model picker.
    fn spawn_ollama_probe(&self) {
        let tx = self.events_tx.clone();
        let base_url = self.config.ollama.base_url.clone();
        tokio::spawn(async move {
            let avail = ollama::detect(&base_url).await;
            let _ = tx.send(AppEvent::OllamaAvailability(avail)).await;
        });
    }

    fn handle_model_picker_key(&mut self, key: KeyEvent) {
        let Some(picker) = self.model_picker.as_mut() else {
            return;
        };
        // Esc behavior is contextual: clear filter first, only close on a
        // second Esc. Lets the user undo a typo without losing the modal.
        match key.code {
            KeyCode::Esc => {
                if picker.filter.is_empty() {
                    self.model_picker = None;
                } else {
                    picker.clear_filter();
                }
            }
            KeyCode::Down => picker.move_selection(1),
            KeyCode::Up => picker.move_selection(-1),
            KeyCode::PageDown => picker.move_selection(10),
            KeyCode::PageUp => picker.move_selection(-10),
            KeyCode::Backspace => picker.pop_char(),
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
            // Any printable char (including space, j/k/q) is treated as filter
            // input — we deliberately don't reserve Vim-style nav here so the
            // user can search for "qwen", "kr" etc. without hitting reserved keys.
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                picker.push_char(c);
            }
            _ => {}
        }
    }

    /// Open the Settings modal. Always works (even when Ollama is down — the
    /// modal will surface the unreachable state).
    fn open_settings(&mut self) {
        self.settings = Some(SettingsState::default());
    }

    fn handle_settings_key(&mut self, key: KeyEvent) {
        let Some(state) = self.settings.as_mut() else {
            return;
        };

        // Editing-text mode owns most keys.
        if let Some(editor) = state.editor.as_mut() {
            match key.code {
                KeyCode::Esc => {
                    state.editor = None;
                }
                KeyCode::Enter => {
                    let edited = state.editor.take().unwrap();
                    self.commit_settings_edit(edited);
                }
                KeyCode::Backspace => editor.backspace(),
                KeyCode::Left => editor.left(),
                KeyCode::Right => editor.right(),
                KeyCode::Home => editor.home(),
                KeyCode::End => editor.end(),
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    editor.insert(c);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.settings = None;
            }
            KeyCode::Char('j') | KeyCode::Down => state.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => state.move_selection(-1),
            KeyCode::Enter => {
                let row = state.selected;
                self.activate_settings_row(row);
            }
            _ => {}
        }
    }

    /// Triggered when user presses Enter on (or clicks) a settings row.
    fn activate_settings_row(&mut self, row: SettingsRow) {
        match row {
            SettingsRow::Model => {
                self.settings = None;
                self.open_model_picker();
            }
            SettingsRow::BaseUrl => {
                if let Some(state) = self.settings.as_mut() {
                    state.editor = Some(FieldEditor::new(
                        SettingsRow::BaseUrl,
                        self.config.ollama.base_url.clone(),
                    ));
                }
            }
            SettingsRow::SystemPrompt => {
                if let Some(state) = self.settings.as_mut() {
                    state.editor = Some(FieldEditor::new(
                        SettingsRow::SystemPrompt,
                        self.config.ollama.system_prompt.clone(),
                    ));
                }
            }
        }
    }

    fn handle_settings_click(&mut self, (x, y): (u16, u16)) {
        let Some(state) = self.settings.as_ref() else {
            return;
        };
        // While editing a field, ignore clicks (Esc/Enter exits the editor).
        if state.editor.is_some() {
            return;
        }
        let hit_row = state
            .row_rects
            .iter()
            .find(|(_, r)| hit(*r, x, y))
            .map(|(r, _)| *r);
        if let Some(row) = hit_row {
            if let Some(s) = self.settings.as_mut() {
                s.selected = row;
            }
            self.activate_settings_row(row);
        }
    }

    /// Persist a finished inline edit back into config + disk.
    fn commit_settings_edit(&mut self, edited: FieldEditor) {
        match edited.row {
            SettingsRow::BaseUrl => {
                let trimmed = edited.buf.trim().to_string();
                if trimmed.is_empty() {
                    self.toast = Some("base URL must not be empty".to_string());
                    return;
                }
                self.config.ollama.base_url = trimmed;
            }
            SettingsRow::SystemPrompt => {
                self.config.ollama.system_prompt = edited.buf;
            }
            SettingsRow::Model => return,
        }
        match config::save(&self.config) {
            Ok(()) => {
                self.toast = Some("settings saved".to_string());
            }
            Err(e) => {
                self.toast = Some(format!("save failed: {e}"));
            }
        }
    }

    /// Open the generation modal directly in error state — used when a
    /// preflight check fails before we ever talk to Ollama, so the failure
    /// shows in the same modal the success path uses.
    fn show_generation_error(&mut self, model: String, message: String) {
        let mut g = GenerationDialog::streaming(model);
        g.finish_error(message);
        self.generation = Some(g);
    }

    /// Key dispatch while the generation modal is up. Locks the rest of the
    /// UI: everything else gets ignored until the modal is dismissed.
    async fn handle_generation_key(&mut self, key: KeyEvent) {
        let Some(g) = self.generation.as_ref() else {
            return;
        };
        match &g.phase {
            GenerationPhase::Streaming { .. } => match key.code {
                KeyCode::Esc => self.cancel_generation(),
                _ => {}
            },
            GenerationPhase::Done { .. } => match key.code {
                KeyCode::Enter => {
                    // "Use it" — close the modal; the message is already in
                    // input.buf, the user reviews it inline and Enters again
                    // to actually commit. Two-step keeps it safe.
                    self.generation = None;
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    self.generation = None;
                    self.start_generation().await;
                }
                KeyCode::Esc => {
                    // Discard generated text, go back to normal mode.
                    self.input.clear();
                    self.input_mode = InputMode::Normal;
                    self.generation = None;
                }
                _ => {}
            },
            GenerationPhase::Error { .. } => match key.code {
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    self.generation = None;
                    self.start_generation().await;
                }
                KeyCode::Esc | KeyCode::Enter => {
                    self.generation = None;
                }
                _ => {}
            },
        }
    }

    /// Abort an in-flight generation: cancel the spawned task and close the
    /// modal. Idempotent.
    fn cancel_generation(&mut self) {
        if let Some(handle) = self.gen_task.take() {
            handle.abort();
        }
        self.input.generating = false;
        self.input.clear();
        self.input_mode = InputMode::Normal;
        self.generation = None;
        self.toast = Some("generation cancelled".to_string());
    }

    /// Kick off a streaming Ollama generation. Pre-flight checks the state
    /// (ollama reachable, model selected, something staged); on success enters
    /// commit-input mode with `generating=true` and spawns the generator.
    async fn start_generation(&mut self) {
        // All preflight failures are surfaced through the modal too — silent
        // toasts make it too easy to miss "ollama isn't running".
        let model = match &self.ollama {
            OllamaAvailability::Ready { models } => self
                .config
                .ollama
                .model
                .clone()
                .filter(|m| models.contains(m))
                .or_else(|| models.first().cloned()),
            OllamaAvailability::NotRunning => {
                self.show_generation_error(
                    "(unknown model)".to_string(),
                    "Ollama is not running on this machine — connection refused at the configured base URL.".to_string(),
                );
                return;
            }
            OllamaAvailability::NoModels => {
                self.show_generation_error(
                    "(none)".to_string(),
                    "Ollama is running but has no models installed yet.".to_string(),
                );
                return;
            }
            OllamaAvailability::Unknown => {
                self.show_generation_error(
                    "(probing)".to_string(),
                    "Still probing the local Ollama instance — try again in a moment.".to_string(),
                );
                return;
            }
        };
        let Some(model) = model else {
            self.show_generation_error(
                "(none)".to_string(),
                "Ollama has no models available to use.".to_string(),
            );
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
            self.show_generation_error(
                model.clone(),
                "Nothing staged — stage at least one file before generating a commit message.".to_string(),
            );
            return;
        }

        // Switch the input bar into streaming mode. Preserve `push_after` if
        // the user already opened it via `C` (commit+push).
        let push_after = self.input.push_after;
        self.input_mode = InputMode::Commit;
        self.input.clear();
        self.input.push_after = push_after;
        self.input.generating = true;
        // Open the blocking modal so the user knows the app is busy and
        // can't accidentally click around.
        self.generation = Some(GenerationDialog::streaming(model.clone()));

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
            // Recent subjects act as a style anchor so the model matches this
            // repo's voice (conventional commits, prefix style, tense). Best-
            // effort: empty list on unborn repo or any git error.
            let recent = git::log::recent_subjects(&root, 10).await;
            let style_block = if recent.is_empty() {
                String::new()
            } else {
                let lines = recent
                    .iter()
                    .map(|s| format!("- {s}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("Recent commit subjects from this repo (match this style):\n{lines}\n\n")
            };
            let diff_header = if was_truncated {
                "Diff (truncated for length):"
            } else {
                "Diff:"
            };
            let prompt = format!("{style_block}{diff_header}\n\n{truncated}");

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

    /// Run a write op in a background task; show toast with result; refresh.
    fn run_op_async(&self, label: &str, op: BoxedOp) {
        let tx = self.events_tx.clone();
        let label = label.to_string();
        tokio::spawn(async move {
            match op.await {
                Ok(s) => {
                    // Successes stay on the transient toast — they don't need
                    // to interrupt the user.
                    let _ = tx.send(AppEvent::LoadFailed(s)).await;
                }
                Err(e) => {
                    // Failures get the modal popup treatment so the user sees
                    // the actual git stderr instead of a fading toast.
                    let _ = tx
                        .send(AppEvent::OpFailed {
                            label: label.clone(),
                            error: e.to_string(),
                        })
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

    /// Visible row count of the active pane's list area. Used by PageUp/Down
    /// and Ctrl-U/D so paging scales with the terminal size instead of using
    /// a fixed step. Returns 0 before the first frame has rendered.
    fn active_viewport_height(&self) -> usize {
        let r = match self.active_pane {
            Pane::Branches => self.last_rects.branches_list,
            Pane::Changes => self.last_rects.changes_list,
            Pane::Graph => self.last_rects.graph_list,
        };
        r.height as usize
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
    ///
    /// Called from user-driven sites (selection moves, pane switches, mouse
    /// clicks) — it always unpins so navigation drops the user back into the
    /// per-selection preview after a "view all" pin.
    fn update_preview(&mut self) {
        self.preview_pinned = false;
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

    /// Background-event variant — used after async refreshes (status/commits
    /// reload) so a pinned preview (e.g. "view all changes") survives.
    fn refresh_preview_after_reload(&mut self) {
        if !self.preview_pinned {
            self.update_preview();
        }
    }

    /// Build the combined diff body for "view all changes" and pin it as the
    /// preview. Selection moves / pane switches / row clicks unpin
    /// automatically via `update_preview`.
    fn show_all_changes_preview(&mut self) {
        let total = self.status.files.len();
        let title = format!("All changes ({total} files)");
        let target = format!("all-changes:{}", self.repo.root.display());

        self.preview_target = Some(target.clone());
        self.preview = Some(DetailsContent::Loading { title: title.clone() });
        self.preview_scroll = 0;
        self.preview_pinned = true;

        let untracked: Vec<String> = self
            .status
            .files
            .iter()
            .filter(|f| matches!(f.kind, ChangeKind::Untracked))
            .map(|f| f.path.clone())
            .collect();
        let root = self.repo.root.clone();
        let tx = self.events_tx.clone();
        tokio::spawn(async move {
            let content = match git::diff::all_changes(&root, &untracked).await {
                Ok(body) => DetailsContent::Body { title, body },
                Err(e) => DetailsContent::Error {
                    title,
                    message: format!("{e}"),
                },
            };
            let _ = tx.send(AppEvent::PreviewLoaded(target, content)).await;
        });
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
        self.graph_layout = None;
        self.graph_truncated = false;
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
                Some(name) => {
                    git::log::fetch_branch(&root, name, upstream.as_deref(), MAX_GRAPH_COMMITS)
                        .await
                }
                None => git::log::fetch(&root, MAX_GRAPH_COMMITS).await,
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
            // PR data is enhancement-only — branches/commits/diffs all work
            // without it. We refresh at most once a minute (well under
            // GitHub's 5000/hr authed budget), so most failures here are
            // transient: 5xx gateway timeouts, network blips, rate-limited
            // CI runners. Toasting every transient blip is more noise than
            // signal; cached PR chips stay visible until the next refresh
            // succeeds. Persistent failures still surface on the next manual
            // `r` refresh path or via gh availability changes.
            if let Ok(map) = gh::fetch_prs(&root).await {
                let _ = tx.send(AppEvent::PrsLoaded(map)).await;
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

/// Best-effort one-line hint for the user based on a failed git op's label
/// and stderr. Pattern-matches common cases (worktree-blocked deletes,
/// non-fast-forward push, dirty checkout) so the modal is actionable, not
/// just a stack trace. Returns None when no useful suggestion applies.
fn suggest_op_hint(label: &str, error: &str) -> Option<String> {
    let lower = error.to_ascii_lowercase();
    let label_lower = label.to_ascii_lowercase();

    // git refuses to delete a branch that's checked out somewhere — including
    // another worktree. The stderr usually mentions the worktree path.
    if label_lower.starts_with("delete") && (lower.contains("checked out at") || lower.contains("worktree")) {
        return Some(
            "This branch is checked out in another worktree. Remove the worktree first \
             with `git worktree remove <path>`, then try the delete again."
                .to_string(),
        );
    }
    if label_lower.starts_with("delete") && lower.contains("not fully merged") {
        return Some(
            "Branch has commits not yet merged into HEAD. If you really want to drop \
             them, force-delete with capital D (`-D`) instead of `d` — but make sure \
             those commits are reachable from another ref first."
                .to_string(),
        );
    }
    if label_lower.contains("push") && (lower.contains("non-fast-forward") || lower.contains("rejected")) {
        return Some(
            "Remote has commits you don't have. Pull (P) first to fast-forward, or \
             force-push only if you're sure no one else is on this branch."
                .to_string(),
        );
    }
    if label_lower.contains("checkout") && lower.contains("would be overwritten") {
        return Some(
            "Local changes would be lost. Commit or stash them first, then retry the \
             checkout."
                .to_string(),
        );
    }
    if lower.contains("permission denied") || lower.contains("publickey") {
        return Some(
            "Git auth failed. Check your SSH agent / credentials helper before retrying."
                .to_string(),
        );
    }
    None
}

/// True when the file is currently treated as staged for toggle purposes —
/// any staged content counts, except for untracked/ignored entries. Used by
/// the per-row toggle button to choose between `+` (stage) and `−` (unstage).
/// Mirrors the historical `toggle_stage_selected` heuristic.
pub(crate) fn is_fully_staged(file: &git::FileChange) -> bool {
    file.staged.is_some()
        && !matches!(
            file.kind,
            crate::git::ChangeKind::Untracked | crate::git::ChangeKind::Ignored
        )
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

/// Cap a staged diff before sending to Ollama. Sized to fit comfortably
/// inside the 32K context window we configure for the model (see
/// `ollama::client`), with headroom for the system prompt, the recent-
/// subject style block, and a multi-paragraph generated body. The boolean
/// signals whether truncation actually happened so the prompt can disclose
/// it; in practice almost no real-world diff hits this.
const MAX_DIFF_LINES: usize = 32_000;
const MAX_DIFF_BYTES: usize = 96 * 1024;

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

/// Coerce the raw streamed model output into a clean commit message
/// (subject + optional body, separated by a blank line). Strips reasoning
/// blocks and outer code fences, normalizes line endings. No length cap —
/// we trust the prompt; truncating mid-sentence is worse than a long commit.
fn clean_message(raw: &str) -> String {
    // 1. Strip reasoning blocks some models emit (Gemma-Thinking,
    //    Qwen-Reasoning, DeepSeek-R1, ...). The answer lives outside.
    let stripped = strip_thinking_blocks(raw).replace("\r\n", "\n");

    // 2. Drop pure code-fence lines from the start and end. Some models wrap
    //    the whole answer in ```...``` even when asked not to.
    let lines: Vec<&str> = stripped.lines().collect();
    let mut start = 0usize;
    let mut end = lines.len();
    while start < end && (lines[start].trim().is_empty() || is_fence_line(lines[start])) {
        start += 1;
    }
    while end > start && (lines[end - 1].trim().is_empty() || is_fence_line(lines[end - 1])) {
        end -= 1;
    }
    let body = lines[start..end].join("\n");

    // 3. If single-line, also strip surrounding quote/backtick characters
    //    that some models still add ("feat: x" → feat: x).
    if !body.contains('\n') {
        body.trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
            .trim()
            .to_string()
    } else {
        body
    }
}

/// True for lines that are nothing but a code-fence marker (``` or ```rust).
fn is_fence_line(line: &str) -> bool {
    let t = line.trim();
    if !t.starts_with("```") {
        return false;
    }
    // After the leading ```, only an optional language tag (alphanumerics).
    t[3..].chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Drop `<think>…</think>` and `<thinking>…</thinking>` regions (case-
/// insensitive). Unclosed openers drop the rest — anything past an unclosed
/// tag is incomplete reasoning and not a usable answer.
fn strip_thinking_blocks(raw: &str) -> String {
    const TAGS: &[(&str, &str)] = &[("<think>", "</think>"), ("<thinking>", "</thinking>")];
    let lower = raw.to_ascii_lowercase();
    let mut out = String::with_capacity(raw.len());
    let mut cursor = 0usize;

    loop {
        // Earliest opener wins, regardless of which tag flavour.
        let next = TAGS
            .iter()
            .filter_map(|(open, close)| {
                lower[cursor..]
                    .find(open)
                    .map(|p| (cursor + p, *open, *close))
            })
            .min_by_key(|(p, _, _)| *p);

        let Some((open_idx, open_tag, close_tag)) = next else {
            out.push_str(&raw[cursor..]);
            break;
        };
        out.push_str(&raw[cursor..open_idx]);

        let after_open = open_idx + open_tag.len();
        match lower[after_open..].find(close_tag) {
            Some(close) => cursor = after_open + close + close_tag.len(),
            None => return out,
        }
    }
    out
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

#[cfg(test)]
mod clean_message_tests {
    use super::{clean_message, strip_thinking_blocks};

    #[test]
    fn keeps_simple_subject() {
        assert_eq!(
            clean_message("feat: add view-all toolbar button"),
            "feat: add view-all toolbar button"
        );
    }

    #[test]
    fn strips_surrounding_quotes_on_single_line() {
        assert_eq!(clean_message("\"feat: x\""), "feat: x");
        assert_eq!(clean_message("`fix: y`"), "fix: y");
    }

    #[test]
    fn ignores_leading_blank_lines() {
        assert_eq!(clean_message("\n\n   \nfeat: hello\n"), "feat: hello");
    }

    #[test]
    fn strips_outer_code_fences() {
        let raw = "```\nfeat: wrapped in fence\n```";
        assert_eq!(clean_message(raw), "feat: wrapped in fence");
    }

    #[test]
    fn strips_outer_language_tagged_fences() {
        let raw = "```text\nfeat: subject\n\nbody line\n```";
        assert_eq!(clean_message(raw), "feat: subject\n\nbody line");
    }

    #[test]
    fn preserves_subject_blank_body_structure() {
        let raw = "feat: add view-all\n\nAdds a button that shows the combined diff.\nPer-file drill-down still works via row clicks.";
        assert_eq!(
            clean_message(raw),
            "feat: add view-all\n\nAdds a button that shows the combined diff.\nPer-file drill-down still works via row clicks."
        );
    }

    #[test]
    fn drops_think_block_and_keeps_following_message() {
        let raw = "<think>\nThe diff adds a toolbar button.\n</think>\nfeat: add toolbar button\n\nLets the user see all uncommitted diffs at once.";
        assert_eq!(
            clean_message(raw),
            "feat: add toolbar button\n\nLets the user see all uncommitted diffs at once."
        );
    }

    #[test]
    fn returns_empty_when_only_thinking() {
        let raw = "<think>\nplanning planning planning\n</think>\n";
        assert_eq!(clean_message(raw), "");
    }

    #[test]
    fn returns_empty_when_unclosed_thinking_block() {
        let raw = "<think>\nstill reasoning…";
        assert_eq!(clean_message(raw), "");
    }

    #[test]
    fn case_insensitive_thinking_tags() {
        let raw = "<Thinking>plan</Thinking>\nfeat: keep working";
        assert_eq!(clean_message(raw), "feat: keep working");
    }

    #[test]
    fn strip_thinking_blocks_handles_mixed_content() {
        let raw = "before <think>x</think> middle <thinking>y</thinking> after";
        assert_eq!(strip_thinking_blocks(raw), "before  middle  after");
    }

    #[test]
    fn does_not_strip_quotes_on_multiline() {
        // Multi-line messages with intentional quotes inside the body should
        // not be quote-stripped — only single-line subjects get that treatment.
        let raw = "\"feat: subject\"\n\nbody mentions \"a quoted phrase\" inside.";
        assert_eq!(
            clean_message(raw),
            "\"feat: subject\"\n\nbody mentions \"a quoted phrase\" inside."
        );
    }
}
