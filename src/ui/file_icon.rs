//! Shared mapping from a file path to a language icon + brand color. Used by
//! the Changes pane file rows AND the diff Preview pane's file-header bar so
//! they stay visually consistent.
//!
//! # Glyph source
//!
//! The icons are Nerd Font codepoints. Nerd Fonts (https://nerdfonts.com,
//! MIT-licensed) is the open-source project that takes the SVG icon sets you
//! were thinking of — Devicons, Material Design Icons, Seti UI, Font Awesome
//! — and bakes them into a font as glyphs in the Unicode private-use area.
//! That's the standard way TUIs (lazygit, gitui, fzf-tab, oh-my-posh, …) get
//! real language brand marks: a terminal cell can only render a font glyph,
//! not a vector image, so SVGs have to be embedded in a font first.
//!
//! # Rendering
//!
//! The icons render as the actual language logos when the terminal's font is
//! a Nerd Font (e.g. `JetBrainsMono Nerd Font`, `Hack Nerd Font`, `FiraCode
//! Nerd Font`, `MesloLGS NF`). Without a Nerd Font installed and selected in
//! the terminal, these private-use codepoints render as the font's missing-
//! glyph box ("tofu") because no fallback font provides them. See the README
//! for install instructions (`brew install --cask font-jetbrains-mono-nerd-font`
//! on macOS, then point the terminal at "JetBrainsMono Nerd Font").
//!
//! # Codepoint families used here
//!
//! | Prefix             | Range          | Source                       |
//! |--------------------|----------------|------------------------------|
//! | `nf-md-*`          | U+F0000+       | Material Design Icons        |
//! | `nf-dev-*`         | U+E700–U+E8FF  | Devicons (vorillaz/devicons) |
//! | `nf-seti-*`        | U+E600–U+E6FF  | Seti UI                      |
//! | `nf-fa-*`          | U+F000–U+F2FF  | Font Awesome 4               |

use ratatui::style::{Color, Style};

use crate::ui::theme::Theme;

/// Pick a language icon + colour for `path`. Match order is `lowercase
/// basename` first (so `Cargo.toml`, `Dockerfile`, `Makefile` win over a
/// generic extension match) → extension → generic file glyph.
pub fn file_icon(path: &str, theme: &Theme) -> (&'static str, Style) {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(lower.as_str());
    let ext = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");

    let (icon, color) = match name {
        // Build / package files — match before extension so they don't lose
        // to a generic .toml / .json / .lock arm.
        "cargo.toml" | "cargo.lock" => ("\u{e7a8}", Color::Rgb(0xDE, 0xA5, 0x84)), // nf-dev-rust
        "package.json" | "package-lock.json" | "npm-shrinkwrap.json" => {
            ("\u{e71e}", Color::Rgb(0xCB, 0x34, 0x37))                              // nf-dev-npm
        }
        "yarn.lock" => ("\u{e6a7}", Color::Rgb(0x2C, 0x8E, 0xBB)),                  // nf-seti-yarn
        "pnpm-lock.yaml" => ("\u{e865}", Color::Rgb(0xF6, 0x9B, 0x36)),             // nf-md-package_variant
        "bun.lockb" | "bun.lock" => ("\u{e76f}", Color::Rgb(0xFB, 0xF0, 0xDF)),     // nf-dev-bower (closest)
        "dockerfile" | "containerfile" | "dockerfile.dev" | "dockerfile.prod" => {
            ("\u{e7b0}", Color::Rgb(0x24, 0x96, 0xED))                              // nf-dev-docker
        }
        "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml" => {
            ("\u{e7b0}", Color::Rgb(0x24, 0x96, 0xED))                              // nf-dev-docker
        }
        "makefile" | "gnumakefile" | "bsdmakefile" => {
            ("\u{e779}", Color::Rgb(0xB8, 0xB8, 0xB8))                              // nf-dev-gnu
        }
        "cmakelists.txt" => ("\u{e794}", Color::Rgb(0x06, 0x49, 0x9B)),             // nf-dev-cmake
        "readme" | "readme.md" | "readme.txt" | "readme.rst" | "readme.markdown" => {
            ("\u{f02d}", Color::Rgb(0x56, 0x9C, 0xD6))                              // nf-fa-book
        }
        "license" | "license.md" | "license.txt" | "licence" | "copying" | "copying.lesser" => {
            ("\u{f718}", theme.fg_dim)                                              // nf-mdi-scale_balance
        }
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitkeep" => {
            ("\u{e702}", Color::Rgb(0xF0, 0x50, 0x33))                              // nf-dev-git
        }
        ".env" | ".env.local" | ".env.production" | ".env.development" | ".env.example"
        | ".env.test" => ("\u{f023}", Color::Rgb(0xE3, 0xCF, 0x55)),                // nf-fa-lock
        ".editorconfig" => ("\u{e615}", theme.fg_dim),                              // nf-seti-config
        ".prettierrc" | ".prettierrc.json" | ".prettierrc.yml" | ".prettierrc.yaml"
        | ".prettierrc.js" => ("\u{e6b4}", Color::Rgb(0xC5, 0x96, 0xC7)),           // nf-seti-prettier
        ".eslintrc" | ".eslintrc.json" | ".eslintrc.js" | ".eslintrc.yml"
        | ".eslintrc.cjs" => ("\u{e655}", Color::Rgb(0x4B, 0x32, 0xC3)),            // nf-seti-eslint
        _ => match ext {
            // ── Systems & compiled languages ───────────────────────────────
            "rs" => ("\u{e7a8}", Color::Rgb(0xDE, 0xA5, 0x84)),                     // nf-dev-rust
            "go" => ("\u{e627}", Color::Rgb(0x00, 0xAD, 0xD8)),                     // nf-dev-go
            "c" | "h" => ("\u{f0671}", Color::Rgb(0x59, 0x9E, 0xD8)),               // nf-md-language_c
            "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "c++" => {
                ("\u{f0672}", Color::Rgb(0x00, 0x59, 0x9C))                         // nf-md-language_cpp
            }
            "cs" => ("\u{f031b}", Color::Rgb(0x68, 0x2A, 0xD7)),                    // nf-md-language_csharp
            "swift" => ("\u{f06e5}", Color::Rgb(0xF0, 0x51, 0x38)),                 // nf-md-language_swift
            "zig" => ("\u{e6a9}", Color::Rgb(0xF7, 0xA4, 0x1D)),                    // nf-seti-zig
            "v" | "vsh" => ("\u{e6ac}", Color::Rgb(0x53, 0x7D, 0xBF)),              // nf-seti-v
            "nim" | "nims" => ("\u{e677}", Color::Rgb(0xFF, 0xC2, 0x00)),           // nf-seti-nim
            "d" | "di" => ("\u{e7af}", Color::Rgb(0xB0, 0x30, 0x31)),               // nf-dev-d (uses dlang glyph)
            "cr" => ("\u{e62f}", Color::Rgb(0xC8, 0xC8, 0xC8)),                     // nf-seti-crystal

            // ── JVM family ─────────────────────────────────────────────────
            "java" => ("\u{e738}", Color::Rgb(0xF8, 0x98, 0x20)),                   // nf-dev-java
            "kt" | "kts" => ("\u{f1219}", Color::Rgb(0xB1, 0x25, 0xEA)),            // nf-md-language_kotlin
            "scala" | "sc" => ("\u{e737}", Color::Rgb(0xDC, 0x32, 0x2D)),           // nf-dev-scala
            "groovy" | "gradle" => ("\u{e7e4}", Color::Rgb(0x02, 0x90, 0x3D)),      // nf-dev-groovy
            "clj" | "cljs" | "cljc" | "edn" => {
                ("\u{e768}", Color::Rgb(0x91, 0xDC, 0x47))                          // nf-dev-clojure
            }

            // ── Functional / academic ──────────────────────────────────────
            "hs" | "lhs" | "cabal" => ("\u{e61f}", Color::Rgb(0x5D, 0x4F, 0x85)),   // nf-seti-haskell
            "ml" | "mli" => ("\u{e67a}", Color::Rgb(0xEC, 0x67, 0x13)),             // nf-seti-ocaml
            "fs" | "fsi" | "fsx" => ("\u{e7a7}", Color::Rgb(0x37, 0x8B, 0xBA)),     // nf-dev-fsharp
            "ex" | "exs" | "eex" | "leex" | "heex" => {
                ("\u{e62d}", Color::Rgb(0x4B, 0x27, 0x5F))                          // nf-seti-elixir
            }
            "erl" | "hrl" => ("\u{e7b1}", Color::Rgb(0xA9, 0x00, 0x33)),            // nf-dev-erlang
            "elm" => ("\u{e62c}", Color::Rgb(0x60, 0xB5, 0xCC)),                    // nf-seti-elm

            // ── Scripting ──────────────────────────────────────────────────
            "py" | "pyi" | "pyc" | "pyw" | "pyx" | "pxd" => {
                ("\u{f0320}", Color::Rgb(0xFF, 0xD4, 0x3B))                         // nf-md-language_python
            }
            "rb" | "erb" | "rake" | "gemspec" | "ru" => {
                ("\u{e739}", Color::Rgb(0xCC, 0x34, 0x2D))                          // nf-dev-ruby
            }
            "php" | "phtml" | "phar" => ("\u{f031f}", Color::Rgb(0x77, 0x7B, 0xB4)),// nf-md-language_php
            "pl" | "pm" | "t" => ("\u{e769}", Color::Rgb(0x39, 0x45, 0x7E)),        // nf-dev-perl
            "lua" => ("\u{e620}", Color::Rgb(0x00, 0x00, 0x80)),                    // nf-seti-lua
            "r" | "rmd" => ("\u{e68a}", Color::Rgb(0x18, 0x65, 0x97)),              // nf-seti-r
            "jl" => ("\u{e624}", Color::Rgb(0xA2, 0x70, 0xBA)),                     // nf-seti-julia
            "dart" => ("\u{e64c}", Color::Rgb(0x03, 0x55, 0x9D)),                   // nf-seti-dart

            // ── Web — JS / TS family ───────────────────────────────────────
            "ts" => ("\u{f06e6}", Color::Rgb(0x31, 0x78, 0xC6)),                    // nf-md-language_typescript
            "tsx" => ("\u{e7ba}", Color::Rgb(0x61, 0xDA, 0xFB)),                    // nf-dev-react
            "js" | "mjs" | "cjs" => ("\u{f031e}", Color::Rgb(0xF7, 0xDF, 0x1E)),    // nf-md-language_javascript
            "jsx" => ("\u{e7ba}", Color::Rgb(0x61, 0xDA, 0xFB)),                    // nf-dev-react
            "vue" => ("\u{fd42}", Color::Rgb(0x41, 0xB8, 0x83)),                    // nf-md-vuejs
            "svelte" => ("\u{e697}", Color::Rgb(0xFF, 0x3E, 0x00)),                 // nf-seti-svelte
            "astro" => ("\u{e6b3}", Color::Rgb(0xFF, 0x5D, 0x01)),                  // nf-md-rocket-launch (proxy)

            // ── Web — markup / styling ─────────────────────────────────────
            "html" | "htm" | "xhtml" => {
                ("\u{f05c0}", Color::Rgb(0xE3, 0x4C, 0x26))                         // nf-md-language_html5
            }
            "css" => ("\u{f031c}", Color::Rgb(0x56, 0x9C, 0xD6)),                   // nf-md-language_css3
            "scss" | "sass" => ("\u{e74b}", Color::Rgb(0xC6, 0x53, 0x8C)),          // nf-dev-sass
            "less" => ("\u{e758}", Color::Rgb(0x1D, 0x36, 0x5D)),                   // nf-dev-less
            "styl" | "stylus" => ("\u{e600}", Color::Rgb(0xB3, 0xD1, 0x07)),        // nf-seti-stylus

            // ── Data / config ──────────────────────────────────────────────
            "json" | "json5" | "jsonc" => ("\u{e60b}", Color::Rgb(0xF7, 0xDF, 0x1E)),// nf-seti-json
            "yml" | "yaml" => ("\u{e6a8}", Color::Rgb(0xCB, 0x17, 0x1E)),           // nf-seti-yml
            "toml" => ("\u{e6b2}", Color::Rgb(0x9C, 0x4A, 0x21)),                   // nf-seti-toml
            "ini" | "cfg" | "conf" | "properties" => {
                ("\u{e615}", theme.fg_dim)                                          // nf-seti-config
            }
            "xml" | "plist" => ("\u{e619}", Color::Rgb(0xE3, 0x4C, 0x26)),          // nf-seti-xml
            "sql" | "psql" | "mysql" | "sqlite" => {
                ("\u{e706}", Color::Rgb(0x56, 0x9C, 0xD6))                          // nf-dev-database
            }
            "csv" | "tsv" => ("\u{f0c7}", Color::Rgb(0x21, 0x9F, 0x4C)),            // nf-fa-table
            "graphql" | "gql" => ("\u{e662}", Color::Rgb(0xE5, 0x35, 0xAB)),        // nf-seti-graphql
            "proto" => ("\u{e60b}", Color::Rgb(0x42, 0x85, 0xF4)),                  // nf-seti-json (proxy)

            // ── Smart contracts ────────────────────────────────────────────
            "sol" => ("\u{f1b3}", Color::Rgb(0xAA, 0x65, 0x46)),                    // nf-fa-cube

            // ── Markup / docs ──────────────────────────────────────────────
            "md" | "markdown" | "mdx" => {
                ("\u{f0354}", Color::Rgb(0x56, 0x9C, 0xD6))                         // nf-md-language_markdown
            }
            "tex" | "latex" | "ltx" | "bib" => {
                ("\u{e69b}", Color::Rgb(0x00, 0x80, 0x00))                          // nf-seti-tex
            }
            "rst" => ("\u{e73e}", theme.fg_dim),                                    // nf-dev-markdown (proxy)
            "org" => ("\u{e633}", Color::Rgb(0x77, 0xAA, 0x99)),                    // nf-custom-emacs
            "adoc" | "asciidoc" => ("\u{f0354}", Color::Rgb(0xE4, 0x09, 0x32)),     // nf-md-language_markdown
            "txt" | "log" => ("\u{f15c}", theme.fg_dim),                            // nf-fa-file_text_o

            // ── Shell / build / ops ────────────────────────────────────────
            "sh" | "bash" | "zsh" | "fish" | "ksh" => {
                ("\u{e795}", Color::Rgb(0x89, 0xE0, 0x51))                          // nf-dev-terminal
            }
            "ps1" | "psm1" | "psd1" => ("\u{ebc7}", Color::Rgb(0x01, 0x21, 0x52)),  // nf-md-powershell
            "bat" | "cmd" => ("\u{f17a}", theme.fg_dim),                            // nf-fa-windows
            "cmake" | "mk" => ("\u{e794}", Color::Rgb(0xB8, 0xB8, 0xB8)),           // nf-dev-cmake
            "ninja" => ("\u{f085}", Color::Rgb(0xCC, 0xCC, 0xCC)),                  // nf-fa-cogs
            "tf" | "tfvars" | "hcl" => ("\u{f1062}", Color::Rgb(0x62, 0x3C, 0xE4)), // nf-md-terraform
            "nix" => ("\u{f313}", Color::Rgb(0x7E, 0xBA, 0xE4)),                    // nf-linux-nixos

            // ── Images ─────────────────────────────────────────────────────
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "webp" | "tiff" | "tif"
            | "heic" | "heif" | "avif" => ("\u{f1c5}", Color::Rgb(0xC2, 0x95, 0xC2)),// nf-fa-file_image_o
            "svg" => ("\u{f1c5}", Color::Rgb(0xFF, 0xB1, 0x3B)),                    // nf-fa-file_image_o

            // ── Audio / video ──────────────────────────────────────────────
            "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus" => {
                ("\u{f1c7}", Color::Rgb(0xE3, 0x49, 0x39))                          // nf-fa-file_audio_o
            }
            "mp4" | "mov" | "avi" | "mkv" | "webm" | "m4v" | "flv" | "wmv" => {
                ("\u{f1c8}", Color::Rgb(0xC4, 0x9D, 0x75))                          // nf-fa-file_video_o
            }

            // ── Archives / binaries ────────────────────────────────────────
            "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "rar" | "7z" | "zst" => {
                ("\u{f1c6}", Color::Rgb(0xB1, 0x9E, 0x7C))                          // nf-fa-file_archive_o
            }
            "exe" | "dll" | "so" | "dylib" | "a" | "lib" | "o" | "obj" => {
                ("\u{f471}", theme.fg_dim)                                          // nf-oct-file_binary
            }
            "wasm" => ("\u{e6a1}", Color::Rgb(0x65, 0x4F, 0xF0)),                   // nf-seti-wasm

            // ── Fonts ──────────────────────────────────────────────────────
            "ttf" | "otf" | "woff" | "woff2" | "eot" => ("\u{f031}", theme.fg_dim), // nf-fa-font

            // ── Office / publishing ────────────────────────────────────────
            "pdf" => ("\u{f1c1}", Color::Rgb(0xE3, 0x49, 0x39)),                    // nf-fa-file_pdf_o
            "doc" | "docx" => ("\u{f1c2}", Color::Rgb(0x29, 0x58, 0xA3)),           // nf-fa-file_word_o
            "xls" | "xlsx" => ("\u{f1c3}", Color::Rgb(0x21, 0x9F, 0x4C)),           // nf-fa-file_excel_o
            "ppt" | "pptx" => ("\u{f1c4}", Color::Rgb(0xD0, 0x4A, 0x37)),           // nf-fa-file_powerpoint_o

            // ── Editor / VCS / misc ────────────────────────────────────────
            "vim" => ("\u{e62b}", Color::Rgb(0x01, 0x9D, 0x33)),                    // nf-dev-vim
            "patch" | "diff" => ("\u{f440}", Color::Rgb(0x73, 0xC9, 0x91)),         // nf-oct-diff
            "bak" | "tmp" | "swp" | "swo" => ("\u{f014}", theme.fg_dim),            // nf-fa-trash_o

            // ── Default — generic file ─────────────────────────────────────
            _ => ("\u{f15b}", theme.fg_dim),                                        // nf-fa-file_o
        },
    };

    (icon, Style::default().fg(color))
}
