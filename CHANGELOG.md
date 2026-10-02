# Changelog

All notable changes to Rustidian are documented here.

## [Unreleased]

### Added

- Recursive vault scanning and **subfolder support**.
- `vault::FolderNode` and `vault::list_notes_tree` for building a folder tree.
- `vault::create_note_in` (create a note inside a folder) and folder-aware
  `rename_note`.
- Expandable **folder tree** in the sidebar, with persisted expansion state and
  a selected target folder for new notes.
- Block-based **Markdown preview**: `markdown::parse_blocks` builds a
  `Block`/`Inline` tree consumed by dedicated Slint components (headings,
  paragraphs, lists, task lists with real checkboxes, code blocks, nested
  quotes, tables with a real `GridLayout`, horizontal rules).
- Inline rendering (bold, italic, strikethrough, inline code, links) through
  Slint's `StyledText`.
- **Markdown editing assistance**: auto-continue lists, auto-closing pairs and
  wikilink autocomplete.
- **Catppuccin themes** (Mocha / Latte) via `ui/theme.slint`, toggled with the
  toolbar button or `Ctrl+T` and persisted in `config.toml`.
- Native folder picker using `rfd` (GTK3 backend) for choosing the vault.
- `vault-ejemplo/00 Markdown prueba.md`, an exhaustive Markdown test note.
- **Drag & drop to move notes**: `vault::move_note` in the core plus native
  Slint `DragArea`/`DropArea` in the sidebar. Drop a note onto a folder to move
  it there, or onto empty space to send it back to the vault root. Open tabs
  and the active note follow the new path automatically; collisions are
  reported instead of overwriting.

### Changed

- The preview no longer uses `pulldown_cmark::html::push_html()`.
- The vault picker no longer requires typing a path.
- `Config` gained a `dark_mode` field (defaults to `true` for existing configs).

## [0.1.0]

- Initial prototype: note CRUD, autosave, wikilinks, backlinks, full-text
  search, optional graph view.
