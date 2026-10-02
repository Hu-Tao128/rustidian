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
- **Folder management**: `vault::create_folder` / `rename_folder` /
  `delete_folder`, a "+ New folder" button, inline rename (right-click) and
  delete with confirmation. `list_notes_tree` now includes empty folders.
  Renaming or deleting a folder remaps/drops the tabs and the active note that
  lived inside it.
- **Creation dialog**: clicking "+ New note" / "+ New folder" (or `Ctrl+N`) now
  opens a modal where you type the name; the note/folder is created on submit,
  with validation errors shown inline instead of failing silently.
- **Resizable sidebar**: drag the handle on the sidebar's right edge (160–480 px)
  to fit the window and the amount of content.
- **Collapsible sidebar**: a panel button collapses the sidebar to a slim icon
  rail and expands it back.
- **File-explorer toolbar**: dedicated icon buttons for new note, new folder,
  sort direction (A–Z / Z–A) and collapse/expand all folders, mirroring
  Obsidian's file-explorer header. The bottom rail repeats them while the
  sidebar is collapsed.
- **Right-click context menu** on sidebar rows: Rename, Delete and, for
  folders, New note / New folder inside that folder. This replaces the inline
  "✕" delete buttons on every row.
- **Session restore**: `Config` gained `open_tabs` and `active_note`; open tabs
  and the active note come back on the next launch (missing files are dropped).
- **Flush on close**: pending edits are written synchronously when the window is
  closed, even if the autosave debounce hasn't fired.
- **Wikilink navigation**: `[[Note]]` / `[[Note|alias]]` become `rustidian://`
  links in the preview; clicking one opens the target note (resolved by path or
  title, case-insensitively). External links open in the default browser.

### Changed

- The preview no longer uses `pulldown_cmark::html::push_html()`.
- The vault picker no longer requires typing a path.
- `Config` gained a `dark_mode` field (defaults to `true` for existing configs).
- The sidebar tree now fills the full sidebar height (it used to stop halfway),
  so folders and notes lower in the list are visible without scrolling.
- Folder rows no longer depend on an emoji glyph for their name, so the name
  stays legible with any font.
- **Folders start collapsed** by default; expand one by clicking its arrow.
- **Icons are now SVG** (`ui/icons/`) tinted from the palette, so the sidebar
  buttons and the light/dark theme switch stay clearly visible in both themes
  instead of relying on font glyph coverage.
- The sidebar toggle now lives in the main toolbar (always reachable) instead of
  the sidebar header, where it overflowed.
- The four file-explorer buttons are centered horizontally and the section label
  is clearer (the "drag to move" hint was removed).
- The sidebar search field is now a palette-driven input (`ui/text_field.slint`)
  instead of the std-widgets `LineEdit`, whose system colours made it disappear
  on the light theme.
- Inputs and buttons are all palette-driven now (`ui/text_field.slint`,
  `ui/palette_button.slint`): the create dialog and the inline rename fields no
  longer follow the OS theme, so they stay readable in dark and light mode.
- Renaming saves on Enter **or** when clicking outside the field; Escape
  cancels. The confusing "×" next to the rename field is gone. The create
  dialog also closes on Escape or when clicking outside it.

### Performance

- Saving now re-indexes only the edited note (`links::update_note`) instead of
  rescanning the whole vault, and the preview is debounced (80 ms) instead of
  re-parsing the Markdown on every keystroke.

## [0.1.0]

- Initial prototype: note CRUD, autosave, wikilinks, backlinks, full-text
  search, optional graph view.
