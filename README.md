# gsc — git source control TUI

VS Code's Source Control panel + Git Graph extension, in your terminal. One short command, three panes, full keyboard control.

```
┌─ Branches (5) ──────────────────┬─ Graph (12 commits) ────────────────────────┐
│ * main                ↑2 ↓0     │ ●─╮  acdea64  HEAD → main   Merge feature   │
│   feature             +2 PR#41  │ ● │  3ecf7a8  main: more readme             │
│   wip                           │ │ ●  424b277  feat: more                    │
│                                 │ │ ●  5cce256  feat: add feature.md          │
│                                 │ ●─╯  b628217  second                        │
│                                 │ ●     f957320  init                          │
├─ Changes (2) ───────────────────┤                                              │
│ ● A staged.md                   │                                              │
│ ○ ? work.txt                    │                                              │
└─────────────────────────────────┴──────────────────────────────────────────────┘
 gsc · main · 2 changes · gh: ✓ · pane: branches · ? help · q quit
```

## Install

### From source (works today)

```sh
git clone https://github.com/shreyashguptas/gsc
cd gsc
cargo install --path .
```

`gsc` lands in `~/.cargo/bin/`. Make sure that's on your `$PATH` (it is by default if you installed Rust via rustup).

### Homebrew (planned for v0.2)

```sh
brew install shreyashguptas/tap/gsc
```

Wired up via [`cargo-dist`](https://opensource.axo.dev/cargo-dist/) — formula auto-publishes on tagged releases. See [Roadmap](#roadmap).

### Requirements

- macOS or Linux
- `git` 2.30+ on `$PATH`
- (Optional) [`gh`](https://cli.github.com) for PR / CI integration — `gsc` degrades gracefully if it's not installed
- A terminal with truecolor + Unicode (any modern one: iTerm2, Ghostty, Warp, kitty, Alacritty, macOS Terminal, Windows Terminal)

## Usage

```sh
cd /path/to/any/git/repo
gsc
```

Other modes:

```sh
gsc --path /elsewhere/repo    # operate on a repo outside cwd
gsc -v                        # verbose logs to stderr (redirect to file to inspect)
gsc --help
```

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
| `Enter` | view full commit + diff |
| `o` | open commit on github.com |

## What `gsc` is — and isn't

**Is**:
- A polished, focused replica of VS Code's Source Control panel + Git Graph
- Read + write: stage, commit, push, pull, fetch, checkout, new/delete branch
- GitHub-aware via `gh`, with branch-level PR chips
- Fast: subprocess to system `git`, async I/O on Tokio, never blocks the UI

**Isn't** (for now):
- A replacement for `git` — drop to `git` for rebase, cherry-pick, stash, bisect, submodule ops, etc.
- An interactive rebase TUI (use `lazygit` or `git rebase -i` for that)
- A merge conflict resolver

The philosophy: do the VS Code panel really well, leave the rest to git.

## Architecture (90-second tour)

```
src/
├── main.rs         entry: terminal setup + restore
├── cli.rs          clap argument parsing
├── app.rs          App state + central Update loop
├── event.rs        AppEvent enum + crossterm/tick/signal tasks
├── git/            subprocess wrappers (no libgit2)
│   ├── repo.rs        discovery + HEAD
│   ├── branches.rs    `git for-each-ref` parser
│   ├── log.rs         `git log --all` DAG fetcher
│   ├── status.rs      `git status --porcelain=v2` parser
│   ├── diff.rs        `git diff` / `git show` + line classifier
│   └── ops.rs         stage / commit / push / checkout / branch / merge
├── gh/             `gh` CLI wrappers (PR chips, auth detect)
├── graph/          DAG layout + Unicode glyph rendering
│   ├── lanes.rs       lane allocation algorithm
│   ├── render.rs      ●/│/╮/╭/╯/╰/─ 2-char cell builder
│   └── color.rs       branch-name → palette index (stable colors)
├── ui/
│   ├── view.rs        top-level layout
│   ├── theme.rs       all colors in one place (swap = re-theme)
│   └── panes/         branches, changes, graph, details, confirm, help, commit_input
└── watcher.rs      `notify` watcher on .git/ → instant refresh
```

Three rules the architecture follows:

1. **No libgit2.** `gsc` shells out to system `git`. This means it honors your config, signing keys, hooks, credential helpers, sparse checkouts, and worktrees — for free. It also keeps the binary slim (~6 MB stripped).
2. **Never block the UI.** Every git/gh call is a `tokio::process::Command` spawned as a `tokio::task`; the result comes back through an `mpsc` channel as an `AppEvent`. The render loop reads state and paints; it never awaits.
3. **One Theme, one place.** All colors and styles live in [`src/ui/theme.rs`](src/ui/theme.rs). A future config can swap the whole palette without touching pane code.

## Tests

```sh
cargo test
```

Covers the porcelain v2 status parser, the `git log` parser, the `for-each-ref` branch parser, the diff line classifier, and the lane allocator (linear / merge / multi-tip topologies). `tests/graph_render.rs` is an integration test that prints the rendered glyph grid to stdout — useful for visual debugging:

```sh
cargo test --test graph_render -- --nocapture
```

## Roadmap

v0.2 (next):
- `cargo-dist` for prebuilt binaries + Homebrew tap
- Sub-row "transition" rendering for the graph (matches `git log --graph` exactly on multi-merge rows)
- Stash list pane (`s`)
- Theme presets (Dracula, Nord, GitHub Light, Solarized)
- syntect syntax highlighting in diff bodies

v1.0:
- Conflict-resolver overlay (3-way diff for merge conflicts)
- Interactive rebase TUI (`R` on a commit)
- Cherry-pick, revert from the graph
- `:command` palette for everything

Have an idea? Open an issue.

## License

MIT. See [LICENSE](LICENSE).
