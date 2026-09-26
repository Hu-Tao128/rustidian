# Rustidian

An ultralight Markdown note editor written in Rust + [Slint](https://slint.dev/), designed to run comfortably on older hardware (Core 2 Duo, ~3 GB RAM, software-rendered X11) while remaining fully usable on modern machines.

Plain `.md` files in a plain folder — no database, no proprietary format.

---

## Features

- **Create, edit, rename and delete** Markdown notes
- **Live preview** (toggle with a button or hide to save CPU)
- **Autosave** with a 600 ms debounce — writes to disk only after you pause typing
- **Full-text search** across titles and note contents with a 150 ms debounce
- **Wikilinks** — `[[Note name]]` and `[[Note|alias]]` syntax
- **Backlinks** — each note shows which other notes link to it
- **Keyboard shortcuts** — `Ctrl+S` save, `Ctrl+N` new note
- **Persistent config** — vault path saved to `~/.config/rustidian/config.toml`
- **Optional graph view** — force-directed layout behind a Cargo feature flag; the default binary never compiles or loads `petgraph`

---

## Requirements

- Rust 1.70 or later (`rustup update stable`)
- A working C linker (`build-essential` / `gcc` on Debian-based distros)
- X11 or Wayland display server (Slint uses `winit` + software renderer by default)

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

---

## Running

```sh
# Uses ~/Notes as vault by default (created on first run).
./target/release/rustidian-ui

# Point to a different vault folder via env var.
RUSTIDIAN_VAULT=/path/to/my/notes ./target/release/rustidian-ui
```

The vault path is persisted to `~/.config/rustidian/config.toml` after the first successful run, so you only need the env var once.

---

## Project structure

```
rustidian/
├── rustidian-core/      # Pure logic: vault CRUD, Markdown, links, search
│   └── src/
│       ├── vault.rs     # list / read / write / create / rename / delete notes
│       ├── markdown.rs  # pulldown-cmark wrapper → HTML
│       ├── links.rs     # wikilink parser + bidirectional index
│       ├── search.rs    # case-insensitive full-text search
│       ├── config.rs    # load/save ~/.config/rustidian/config.toml
│       ├── graph.rs     # Fruchterman-Reingold layout (#[cfg(feature="graph")])
│       └── error.rs     # CoreError (thiserror)
│
├── rustidian-ui/        # Slint UI binary
│   ├── src/
│   │   ├── main.rs      # wires core ↔ UI, timers, callbacks
│   │   ├── bridge.rs    # NoteMeta → NoteItem / BacklinkItem conversions
│   │   └── worker.rs    # background threads + slint::invoke_from_event_loop
│   └── ui/
│       ├── main.slint   # AppWindow, toolbar, keyboard shortcuts
│       ├── sidebar.slint
│       ├── editor.slint
│       ├── preview.slint
│       ├── graph_view.slint
│       └── types.slint  # shared NoteItem / BacklinkItem structs
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

- **Renaming a note breaks existing `[[links]]`** — same behaviour as Obsidian without the "update links on rename" plugin. Tracked as a future improvement (stable IDs via YAML frontmatter).
- **No native folder picker** — change the vault via the `RUSTIDIAN_VAULT` env var; a native dialog is planned for a later version.
- **Preview shows raw HTML** — Slint has no built-in HTML renderer; a proper rendered preview would require embedding a WebView.

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

---

## License

MIT — see [`LICENSE`](LICENSE).
