# Rustidian

An ultralight Markdown note editor written in Rust + [Slint](https://slint.dev/), designed to run comfortably on older hardware (Core 2 Duo, ~3 GB RAM, software-rendered X11) while remaining fully usable on modern machines.

Plain `.md` files in a plain folder — no database, no proprietary format.

---

## Download

Prebuilt binaries are published on the
[Releases](https://github.com/Hu-Tao128/rustidian/releases/latest) page:

| Platform | Artifact | Installation |
| --- | --- | --- |
| Debian / Ubuntu | `rustidian_<version>_amd64.deb` | `sudo apt install ./rustidian_<version>_amd64.deb` |
| Fedora / RHEL | `rustidian-<version>-1.x86_64.rpm` | `sudo dnf install ./rustidian-<version>-1.x86_64.rpm` |
| Windows x86_64 | `rustidian-<version>-windows-x86_64.zip` | unzip and run `rustidian-ui.exe` |
| macOS (Apple Silicon / Intel) | `rustidian-<version>-macos-*.dmg` | open the `.dmg` and drag **Rustidian** into *Applications* |

The installed launcher is `rustidian-ui`, and it also shows up as **Rustidian**
in your application menu. Prefer to build it yourself? See
[Building](#building).

---

## Features

- **Create, edit, rename and delete** Markdown notes
- **Subfolders** — the vault is scanned recursively and the sidebar shows an expandable folder tree; create, rename (right-click) and delete folders, including empty ones
- **Move notes by drag & drop** — drag a note onto a folder to move it there, or onto empty space to send it back to the vault root
- **Live preview** rendered from a real block model (headings, paragraphs, lists, task lists, code blocks, nested quotes, tables and horizontal rules) with native bold/italic/strikethrough/inline-code/links via Slint's `StyledText`
- **Autosave** with a 600 ms debounce — writes to disk only after you pause typing
- **Full-text search** across titles and note contents with a 150 ms debounce
- **Wikilinks** — `[[Note name]]` and `[[Note|alias]]` syntax; click one in the preview to open the target note (resolved by path or title)
- **Backlinks** — each note shows which other notes link to it
- **Session restore** — open tabs and the active note come back on the next launch
- **Safe exit** — pending edits are flushed when you close the window, even if the autosave debounce hasn't fired yet
- **Markdown editing assistance**
  - Auto-continue lists (`- item`, `1. item`, `- [ ] task`) on Enter; pressing Enter on an empty item leaves the list
  - Auto-closing pairs for `**`, `_`, `` ` `` and `[[`, wrapping the current selection when there is one
  - Wikilink autocomplete: type `[[` and pick from the live-filtered list of note titles with Tab/Enter
- **Catppuccin themes** — Mocha (dark) and Latte (light), toggled from the toolbar or with `Ctrl+T`, persisted in the config file
- **Native folder picker** — choosing or changing the vault opens the system file explorer instead of typing a path
- **Keyboard shortcuts** — `Ctrl+S` save, `Ctrl+N` new note, `Ctrl+P` cycle view, `Ctrl+T` toggle theme
- **Persistent config** — vault path, theme and open tabs saved to `~/.config/rustidian/config.toml`
- **Incremental link indexing** — saving re-indexes only the edited note and the preview is debounced, instead of rescanning the vault on every keystroke
- **Optional graph view** — force-directed layout behind a Cargo feature flag; the default binary never compiles or loads `petgraph`

---

## Requirements

### 1. Rust toolchain

Rustidian needs **Rust 1.92 or newer** (Slint 1.18 uses the 2024 edition). Install
it with the official `rustup` script — it is **not** installed through the distro
package manager:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Then add the components used by the development workflow (formatting and linting):

```bash
rustup component add rustfmt clippy
```

Check the version with `rustc --version`; it must print `1.92` or later.

### 2. System dependencies

Rustidian links against a few C libraries at build time:

| Library | Needed for |
|---|---|
| C toolchain (`gcc`/`cc`, `make`) | compiling and linking the binary |
| `pkg-config` | locating the libraries below during the build |
| **GTK3** development headers | the native folder picker (`rfd`, GTK3 backend) |
| **xkbcommon** | keyboard input in Slint's `winit` backend |
| **xcb** (`shape` + `xfixes`) | X11 windowing in Slint's `winit` backend |
| **fontconfig** | system-font enumeration in Slint |

Install everything with the command for your distribution:

#### Arch Linux (and derivatives)

```bash
sudo pacman -S --needed base-devel curl pkgconf gtk3 libxkbcommon libxcb fontconfig
```

#### Debian / Ubuntu (and derivatives)

```bash
sudo apt update
sudo apt install -y build-essential curl pkg-config libgtk-3-dev \
    libxkbcommon-dev libxkbcommon-x11-dev \
    libxcb1-dev libxcb-shape0-dev libxcb-xfixes0-dev \
    libfontconfig1-dev
```

#### Fedora / RHEL / CentOS (and derivatives)

```bash
sudo dnf install -y gcc gcc-c++ make curl pkgconf-pkg-config gtk3-devel \
    libxkbcommon-devel libxkbcommon-x11-devel \
    libxcb-devel fontconfig-devel
```

### 3. Display server

Slint renders through `winit` with the software renderer, so an X11 session is
enough — that is the target environment (old laptops running IceWM/X11). Wayland
also works, but then the Wayland client libraries (`libwayland-client`,
`libxkbcommon`) must be present at runtime. No GPU or OpenGL acceleration is
required.

---

## Building

### Default (lightweight) binary

```sh
cargo build --release -p rustidian-ui
```

### With the optional graph view

```sh
cargo build --release -p rustidian-ui --features graph
```

The binary lands in `target/release/rustidian-ui`.

### Release packages

`scripts/build-release.sh` builds and packages the distributables (`.deb`,
`.rpm`, a Windows `.zip` and macOS `.dmg`/`.tar.gz`) and can publish them to a
GitHub Release with the `gh` CLI:

```sh
# Packages only (written to target/distrib/)
./scripts/build-release.sh --deb --rpm --no-release

# Everything and publish a release
./scripts/build-release.sh --all
```

Run `./scripts/build-release.sh --help` for all options. Windows and macOS
artifacts are built by the `ci-release.yml` workflow when they cannot be
produced natively on the local machine.

---

## Running

```sh
# Uses ~/Notes as vault by default (created on first run).
./target/release/rustidian-ui

# Point to a different vault folder via env var.
RUSTIDIAN_VAULT=/path/to/my/notes ./target/release/rustidian-ui
```

On first launch Rustidian asks you to choose a vault folder through the native
file dialog. The choice is persisted to `~/.config/rustidian/config.toml`, so
you only need the env var once.

---

## Project structure

```
rustidian/
├── rustidian-core/      # Pure logic: vault CRUD, Markdown, links, search
│   └── src/
│       ├── vault.rs     # list / read / write / create / rename / delete,
│       │                # recursive scan + FolderNode tree (list_notes_tree)
│       ├── markdown.rs  # pulldown-cmark events -> Block/Inline tree
│       ├── links.rs     # wikilink parser + bidirectional index
│       ├── search.rs    # case-insensitive full-text search
│       ├── config.rs    # load/save ~/.config/rustidian/config.toml
│       ├── graph.rs     # Fruchterman-Reingold layout (#[cfg(feature="graph")])
│       └── error.rs     # CoreError (thiserror)
│
├── rustidian-ui/        # Slint UI binary
│   ├── src/
│   │   ├── main.rs        # wires core ↔ UI, timers, callbacks, theming
│   │   ├── bridge.rs      # core types -> Slint models (blocks + tree)
│   │   ├── editor_assist.rs # list continuation, auto-pairs, wikilinks
│   │   └── worker.rs      # background threads + slint::invoke_from_event_loop
│   └── ui/
│       ├── main.slint     # AppWindow, toolbar, keyboard shortcuts
│       ├── theme.slint    # Catppuccin `global Palette` (only place with hex)
│       ├── sidebar.slint  # expandable folder tree + backlinks
│       ├── editor.slint   # low-level TextInput + editing assistance
│       ├── preview.slint  # renders the BlockItem model
│       ├── heading.slint / paragraph.slint / list_item.slint /
│       │   task_item.slint / code_block.slint / block_quote.slint /
│       │   table_block.slint / thematic_break.slint
│       ├── graph_view.slint
│       └── types.slint    # shared structs and enums
│
└── vault-ejemplo/       # Sample notes for development (never your real vault)
```

**Architecture rule:** `rustidian-core` never imports `slint`. All disk access goes through `core::vault`. The UI never calls `std::fs` directly.

---

## Configuration

| Method | Detail |
|---|---|
| Default vault | `~/Notes` (created automatically on first run) |
| Env var | `RUSTIDIAN_VAULT=/path` — applied once, then persisted |
| Config file | `~/.config/rustidian/config.toml` |
| Theme | `dark_mode = true` (Mocha) / `false` (Latte) |

Example `config.toml`:

```toml
vault_path = "/home/user/Notes"
dark_mode = true
```

---

## Markdown preview model

The preview no longer relies on `pulldown_cmark::html::push_html()`. Instead,
`rustidian_core::markdown::parse_blocks` consumes the parser's event stream and
builds a UI-agnostic tree:

```rust
enum Block {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    List { ordered: bool, items: Vec<Vec<Block>> },
    TaskList(Vec<(bool, Vec<Inline>)>),
    CodeBlock { lang: Option<String>, code: String },
    BlockQuote(Vec<Block>),
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
    ThematicBreak,
}
```

Rust flattens this into a `[BlockItem]` model (nesting expressed with an
`indent` field, tables as a flat cell array plus a column count) and Slint
renders each kind with a dedicated component. Inline spans are serialised to
CommonMark and parsed by Slint's `StyledText` element, which natively renders
bold, italic, strikethrough, inline code and links.

`vault-ejemplo/00 Markdown prueba.md` is a test note that covers every case:
bold, italic, strikethrough, inline code, fenced code with a language, links,
images, nested lists, task lists, nested block quotes, tables and a horizontal
rule.

---

## Themes

All colours live in `ui/theme.slint` (`global Palette`); no other `.slint` file
uses a literal colour. The two themes are Catppuccin Mocha (dark) and Latte
(light):

| Role | Mocha | Latte |
|---|---|---|
| `bg` — main background | `#1e1e2e` | `#eff1f5` |
| `surface` — sidebar / panels | `#181825` | `#e6e9ef` |
| `card` — cards / active tabs | `#313244` | `#ccd0da` |
| `border` — borders / dividers | `#45475a` | `#bcc0cc` |
| `text` — primary text | `#cdd6f4` | `#4c4f69` |
| `text-muted` — secondary text | `#a6adc8` | `#6c6f85` |
| `accent` — buttons, focus, links | `#74c7ec` | `#209fb5` |
| `success` — "Saved" | `#a6e3a1` | `#40a02b` |
| `danger` — delete | `#f38ba8` | `#d20f39` |
| `warning` | `#f9e2af` | `#df8e1d` |

Toggle with the toolbar button or `Ctrl+T`; the choice is stored as
`dark_mode` in `config.toml`.

---

## Development

```sh
# Run all tests
cargo test --workspace

# Check formatting
cargo fmt --check

# Lint (must be warning-free)
cargo clippy --all-targets --all-features -- -D warnings

# Quick dev run against the sample vault
RUSTIDIAN_VAULT=vault-ejemplo cargo run -p rustidian-ui
```

Use `vault-ejemplo/` for development — never point a dev build at your real notes.

---

## Known limitations (v1)

- **Renaming or moving a note — or renaming/deleting a folder that contains notes — breaks existing `[[links]]`** — same behaviour as Obsidian without the "update links on rename" plugin. Tracked as a future improvement (stable IDs via YAML frontmatter).
- **Dropping a note onto a folder where a note with the same name already exists is rejected** — Rustidian shows an error instead of overwriting.
- **Images in the preview are shown as labelled links** — Slint's `StyledText` has no inline image support, so `![alt](url)` renders as a clickable `🖼 alt` link.
- **The graph view is opt-in** — build with `--features graph` on machines that can afford the layout calculation.

---

## Roadmap

| Milestone | Status |
|---|---|
| V0 — skeleton compiles and runs | ✅ done |
| V1 — robust CRUD + autosave | ✅ done |
| V2 — wikilinks + backlinks | ✅ done |
| V3 — full-text search | ✅ done |
| V4 — UX polish (shortcuts, indicators, first-run) | ✅ done |
| V5 — graph view (`--features graph`) | 🔧 scaffolded, layout implemented |
| V6 — subfolders, block preview, editing assistance, themes | ✅ done |
| V7 — drag-and-drop note moving | ✅ done |
| V8 — folder management, session restore, wikilink navigation, incremental indexing | ✅ done |

---

## License

MIT — see [`LICENSE`](LICENSE).
