# gsc — git source control TUI

VS Code's Source Control panel + Git Graph extension, in your terminal. One short command, four panes, full keyboard control. Live diff preview that updates as you arrow through commits.

```
┌─ Branches (3) ───────┬─ Graph · main ↔ origin/main · ↑1 to push · 12 commits ─┬─ Preview · acdea64 …─┐
│ * main      ↑1 ↓0    │ ↑ ●─╮ acdea64 [ ◉ main ] new merge                      │ commit acdea64...    │
│   feature   +2 PR#41 │   ● │ 3ecf7a8 [ ⎇ feature ]☁ wip on feature             │ Author: Shreyash G.  │
│   wip                │   │ ●  424b277  feat: more                              │ Date:   2026-05-02   │
│                      │   ●─╯ b628217 [ ◉ main-old ]☁ ▸ v1.0  release v1.0      │                      │
├─ Changes (2) ────────┤   ●    f957320  init                                    │     new merge        │
│ ● A staged.md        │                                                          │                      │
│ ○ ? work.txt         │                                                          │ diff --git a/...     │
│                      │                                                          │ @@ -1,2 +1,3 @@      │
│                      │                                                          │ +main work           │
└──────────────────────┴──────────────────────────────────────────────────────────┴──────────────────────┘
 gsc · main · 2 changes · gh: ✓ · graph · ↑↓ live preview · Enter full · o github · ? help · q quit
```

## Quick start

Copy-paste this. It installs `gsc` to `~/.cargo/bin/gsc` and runs it on the current repo.

```sh
git clone https://github.com/shreyashguptas/git-source-for-cli.git
cd git-source-for-cli
cargo install --path .
gsc
```

That's it. From any git repo, just type `gsc`.

**Don't have Rust yet?** Install it first (one command, ~1 min):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then re-open your shell (or `source $HOME/.cargo/env`) and run the four commands above.

### Just want to try it without installing?

```sh
git clone https://github.com/shreyashguptas/git-source-for-cli.git
cd git-source-for-cli
cargo run --release
```

### Update to the latest version

```sh
cd git-source-for-cli
git pull
cargo install --path . --force
```

## Requirements

- macOS or Linux
- `git` 2.30+ on `$PATH`
- A terminal with truecolor + Unicode (iTerm2, Ghostty, Warp, kitty, Alacritty, macOS Terminal, Windows Terminal — any modern one)
- (Optional) [`gh`](https://cli.github.com) for PR / CI integration — `gsc` degrades gracefully if it's not installed

## Reading the graph at a glance

- `[ ◉ main ]` — coloured pill with the filled-circle icon: this is your **current HEAD**
- `[ ⎇ feature ]` — coloured pill with the branch icon: a **local-only** branch
- `☁` — cloud chip immediately after a pill: that branch is **also on origin** (in sync). No cloud = unpushed.
- `[ ☁ origin/old ]` — standalone cloud pill: a branch that exists **only on origin**
- `▸ v1.0` — italic pill: a **tag**
- `↑ ●` in the left margin: this commit is **ahead of origin** (will be pushed when you press `p`)
- `↓ ●` in the left margin: this commit is **only on origin** (will arrive when you press `P`)

**The big idea**: arrow up/down on Branches previews that branch/worktree in the Graph without checking it out. Arrow through Graph (or Changes) and the right-side Preview updates instantly. No modal popups for casual browsing — `Enter` opens the full-screen view only when you want more room to scroll. Mouse works too.

## Usage

```sh
gsc                           # run on the current directory's repo
gsc --path /elsewhere/repo    # operate on a repo outside cwd
gsc -v                        # verbose logs to stderr
gsc --help
```

## Mouse

| Action | Result |
|---|---|
| Left-click a pane | focus that pane and select the item under the cursor |
| Click + drag a pane border | resize the panes |
| Scroll wheel inside a pane | scroll that pane |
| `=` key | reset all panes to default proportions |

**macOS tip**: while mouse capture is on, normal text-selection in the terminal is intercepted. Hold **Option** while dragging to bypass capture and select text natively.

## Keybindings

`?` opens the in-app help overlay. The most common ones:

**Global**
| Key | Action |
|---|---|
| `q`  /  `Ctrl-C` | quit |
| `Tab` / `Shift-Tab` | cycle pane |
| `1` / `2` / `3` | jump to Branches / Changes / Graph |
| `j k` `↑↓` `g G` `PgUp PgDn` | navigate |
| `r` | force refresh |
| `?` | help |

**Branches pane**
| Key | Action |
|---|---|
| `↑↓` `j k` | preview selected branch/worktree in the Graph pane |
| `Enter` | checkout selected branch |
| `n` | new branch (type name → `Enter`) |
| `d` / `D` | delete (safe / force) — confirms |
| `m` | merge selected branch into current — confirms |
| `p` / `P` / `f` | push / pull `--ff-only` / fetch all |
| `o` | open PR on github.com (if `gh` is connected) |

**Changes pane**
| Key | Action |
|---|---|
| `Enter` | view diff in overlay |
| `Space` | stage / unstage selected file |
| `a` / `A` | stage all / unstage all |
| `c` / `C` | commit / commit and push (type message → `Enter`) |
| `x` | discard local changes — confirms |

**Graph pane**
| Key | Action |
|---|---|
| `↑↓` `j k` | scroll — Preview pane updates automatically |
| `Enter` | open the diff full-screen |
| `o` | open commit on github.com |

The Preview pane updates automatically as the Graph or Changes selection moves. Hides automatically when the terminal is narrower than 130 cols.

## What `gsc` is — and isn't

**Is**:
- A polished, focused replica of VS Code's Source Control panel + Git Graph
- Read + write: stage, commit, push, pull, fetch, checkout, new/delete branch
- GitHub-aware via `gh`, with branch-level PR chips
- Fast: subprocess to system `git`, async I/O on Tokio, never blocks the UI

**Isn't** (for now):
- A replacement for `git` — drop to `git` for rebase, cherry-pick, stash, bisect, submodule ops
- An interactive rebase TUI (use `lazygit` or `git rebase -i`)
- A merge conflict resolver

## Tests

```sh
cargo test
```

`tests/graph_render.rs` is an integration test that prints the rendered glyph grid to stdout — useful for visual debugging:

```sh
cargo test --test graph_render -- --nocapture
```

## Architecture (60-second tour)

```
src/
├── main.rs         entry: terminal setup + restore
├── cli.rs          clap argument parsing
├── app.rs          App state + central Update loop
├── event.rs        AppEvent enum + crossterm/tick/signal tasks
├── git/            subprocess wrappers (no libgit2)
├── gh/             `gh` CLI wrappers (PR chips, auth detect)
├── graph/          DAG layout + Unicode glyph rendering
├── ui/             panes, theme, top-level layout
└── watcher.rs      `notify` watcher on .git/ → instant refresh
```

Three rules:

1. **No libgit2.** `gsc` shells out to system `git`, so it honors your config, signing keys, hooks, credential helpers, sparse checkouts, and worktrees — for free. Slim binary too (~6 MB stripped).
2. **Never block the UI.** Every git/gh call is a `tokio::process::Command` spawned as a `tokio::task`; results come back through an `mpsc` channel as an `AppEvent`.
3. **One Theme, one place.** All colors live in [`src/ui/theme.rs`](src/ui/theme.rs).

## Roadmap

**v0.2 (next):** Homebrew tap via `cargo-dist`, sub-row graph transitions, stash list pane, theme presets (Dracula, Nord, GitHub Light, Solarized), syntect syntax highlighting in diff bodies.

**v1.0:** Conflict-resolver overlay (3-way diff), interactive rebase TUI, cherry-pick / revert from the graph, `:command` palette.

Have an idea? Open an issue.

## License

MIT. See [LICENSE](LICENSE).
