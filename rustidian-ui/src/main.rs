mod bridge;
mod editor_assist;
mod update;
mod worker;

use arboard::Clipboard;
use bridge::{blocks_to_items, to_backlink_items, tree_to_rows};
use rustidian_core::config::Config;
use rustidian_core::links::normalise;
use rustidian_core::markdown::parse_blocks;
use rustidian_core::vault;
use slint::{Color, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use worker::AppData;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── CLI (--update / --check / --version / --help) ─────────────────────────
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = update::handle_cli(&args) {
        std::process::exit(code);
    }

    // ── Shared state (vault path starts empty) ────────────────────────────────
    let shared: Arc<Mutex<AppData>> = Arc::new(Mutex::new(AppData::default()));

    // ── Build UI ──────────────────────────────────────────────────────────────
    let ui = AppWindow::new()?;
    ui.set_graph_available(cfg!(feature = "graph"));
    ui.set_view_mode(ViewMode::Split);

    // ── Theme ─────────────────────────────────────────────────────────────────
    let config = Config::load().ok();
    let dark = config.as_ref().map(|c| c.dark_mode).unwrap_or(true);
    let check_updates_on_start = config.as_ref().map(|c| c.check_updates).unwrap_or(false);
    ui.set_dark_mode(dark);
    apply_theme(&ui, dark);

    // ── Drag & drop payload helpers ───────────────────────────────────────────
    //
    // Slint's `DragArea`/`DropArea` carry an opaque `data-transfer`.  A note
    // drag is encoded as the note id in plain text; these callbacks build and
    // read that payload.
    {
        let dnd = ui.global::<Dnd>();
        dnd.on_note_transfer(slint::DataTransfer::from);
        dnd.on_transfer_note(|data| data.plain_text().unwrap_or_default());
        dnd.on_can_drop_note(|data| {
            data.plain_text()
                .map(|text| text.ends_with(".md"))
                .unwrap_or(false)
        });
    }

    // ── Wikilink activation from the preview ──────────────────────────────────
    //
    // `StyledText` turns `[[Target]]` into a `rustidian://` link (built by
    // rustidian-core).  Resolve the target to a note and open it.
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.global::<Links>().on_activated(move |url| {
            let Some(target) = rustidian_core::markdown::decode_wikilink_target(url.as_str())
            else {
                return;
            };
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            let resolved = {
                let data = shared2.lock().unwrap();
                resolve_wikilink(&data.notes, &target)
            };
            match resolved {
                Some(id) => ui.invoke_note_selected(id.into()),
                None => {
                    ui.set_status_kind(StatusKind::Error);
                    ui.set_status_message(format!("Note not found: {target}").into());
                }
            }
        });
    }

    // ── Decide whether to show the vault picker ───────────────────────────────
    //
    // Priority:
    //  1. RUSTIDIAN_VAULT env var (skips the picker entirely)
    //  2. Saved config file (~/.config/rustidian/config.toml)
    //  3. Show the picker so the user can choose a folder
    let initial_vault: Option<PathBuf> = {
        if let Ok(ev) = std::env::var("RUSTIDIAN_VAULT") {
            let p = PathBuf::from(ev);
            p.is_dir().then_some(p)
        } else {
            config
                .as_ref()
                .filter(|c| c.vault_path.is_dir())
                .map(|c| c.vault_path.clone())
        }
    };

    if let Some(vault) = initial_vault {
        open_vault(vault.clone(), Arc::clone(&shared), ui.as_weak());
        // Restore the previous session (open tabs + active note) when the saved
        // config belongs to this same vault.
        if let Some(cfg) = config.as_ref().filter(|c| c.vault_path == vault) {
            restore_session(&ui, &vault, &cfg.open_tabs, &cfg.active_note);
        }
        let refresh = make_refresh(ui.as_weak(), Arc::clone(&shared));
        worker::rebuild_index(vault, Arc::clone(&shared), refresh);
    } else {
        ui.set_show_vault_picker(true);
    }

    // ── browse-vault-requested (native folder picker) ─────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_browse_vault_requested(move || {
            let ui_weak = ui_weak.clone();
            let shared2 = Arc::clone(&shared2);
            // The GTK dialog runs on its own thread so the Slint event loop
            // stays responsive.
            std::thread::spawn(move || {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Choose your vault folder")
                    .pick_folder()
                {
                    let ui_weak = ui_weak.clone();
                    let shared2 = Arc::clone(&shared2);
                    slint::invoke_from_event_loop(move || {
                        activate_vault(path, shared2, ui_weak);
                    })
                    .ok();
                }
            });
        });
    }

    // ── vault-open-requested (kept for programmatic opening) ──────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_vault_open_requested(move |path_str| {
            let path = PathBuf::from(path_str.as_str().trim());
            if path.is_dir() {
                activate_vault(path, Arc::clone(&shared2), ui_weak.clone());
            } else if let Some(ui) = ui_weak.upgrade() {
                ui.set_vault_picker_error("That path is not a folder.".into());
            }
        });
    }

    // ── note-selected ─────────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_note_selected(move |id| {
            let id_str = id.to_string();

            // Auto-save the current note before switching so no content is lost,
            // then refresh its links incrementally (no full vault rescan).
            let mut saved_previous: Option<String> = None;
            if let Some(ui) = ui_weak.upgrade() {
                let cur_id = ui.get_current_note_id().to_string();
                let cur_content = ui.get_current_note_content().to_string();
                if !cur_id.is_empty() && ui.get_unsaved() {
                    let vault_path = shared2.lock().unwrap().vault_path.clone();
                    if vault::write_note(&vault_path, &cur_id, &cur_content).is_ok() {
                        saved_previous = Some(cur_id);
                    }
                }
            }
            if let Some(previous_id) = saved_previous {
                let refresh = make_refresh(ui_weak.clone(), Arc::clone(&shared2));
                worker::reindex_note(previous_id, Arc::clone(&shared2), refresh);
            }

            // Read everything we need under a single short-lived lock.
            let (note, bl_items, folder) = {
                let data = shared2.lock().unwrap();
                let note = match vault::read_note(&data.vault_path, &id_str) {
                    Ok(n) => n,
                    Err(e) => {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_status_kind(StatusKind::Error);
                            ui.set_status_message(format!("Error opening note: {e}").into());
                        }
                        return;
                    }
                };
                let folder = std::path::Path::new(&note.meta.id)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let bl_ids = data
                    .link_index
                    .backlinks
                    .get(&normalise(&note.meta.id))
                    .cloned()
                    .unwrap_or_default();
                let bl_items = to_backlink_items(&bl_ids, &data.notes);
                (note, bl_items, folder)
            };

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_current_note_id(note.meta.id.clone().into());
                ui.set_current_note_title(note.meta.title.clone().into());
                ui.set_current_note_content(note.content.clone().into());
                render_preview(&ui, &note.content);
                ui.set_unsaved(false);
                ui.set_status_kind(StatusKind::Success);
                ui.set_status_message(format!("Opened: {}", note.meta.title).into());
                ui.set_backlinks(ModelRc::new(VecModel::from(bl_items)));
                add_or_activate_tab(&ui, &note.meta.id, &note.meta.title);

                // Selecting a note also selects its parent folder so that a new
                // note lands next to it.
                if let Ok(mut data) = shared2.lock() {
                    data.selected_folder = folder;
                }
                if let Ok(data) = shared2.lock() {
                    refresh_sidebar(&ui, &data);
                }
                persist_session(&ui, &shared2);
            }
        });
    }

    // ── folder-toggled (expand/collapse + select) ─────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_folder_toggled(move |path| {
            let path_str = path.to_string();
            if let Ok(mut data) = shared2.lock() {
                let entry = data.expanded.entry(path_str.clone()).or_insert(false);
                *entry = !*entry;
                data.selected_folder = path_str;
            }
            if let Some(ui) = ui_weak.upgrade() {
                if let Ok(data) = shared2.lock() {
                    refresh_sidebar(&ui, &data);
                }
            }
        });
    }

    // ── content-changed (marks unsaved + debounces preview and autosave) ──────
    {
        let ui_weak = ui.as_weak();
        let autosave_timer = Rc::new(Timer::default());
        let preview_timer = Rc::new(Timer::default());
        let shared2 = Arc::clone(&shared);
        ui.on_content_changed(move |text| {
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            ui.set_unsaved(true);

            let cur_id = ui.get_current_note_id().to_string();
            mark_tab_unsaved(&ui, &cur_id, true);

            // Debounce the preview: re-parsing and rebuilding the whole
            // StyledText model on every keystroke is the expensive part on the
            // old hardware this app targets.
            let preview_text = text.to_string();
            let ui_weak_preview = ui_weak.clone();
            preview_timer.start(
                TimerMode::SingleShot,
                std::time::Duration::from_millis(80),
                move || {
                    if let Some(ui) = ui_weak_preview.upgrade() {
                        // Skip if the user has switched notes in the meantime.
                        if ui.get_current_note_content().as_str() == preview_text {
                            render_preview(&ui, &preview_text);
                        }
                    }
                },
            );

            let ui_weak2 = ui_weak.clone();
            let shared3 = Arc::clone(&shared2);
            autosave_timer.start(
                TimerMode::SingleShot,
                std::time::Duration::from_millis(600),
                move || {
                    let ui = match ui_weak2.upgrade() {
                        Some(u) => u,
                        None => return,
                    };
                    let id = ui.get_current_note_id().to_string();
                    let content = ui.get_current_note_content().to_string();
                    if id.is_empty() {
                        return;
                    }
                    let vault = {
                        let data = shared3.lock().unwrap();
                        data.vault_path.clone()
                    };
                    match vault::write_note(&vault, &id, &content) {
                        Ok(()) => {
                            ui.set_unsaved(false);
                            mark_tab_unsaved(&ui, &id, false);
                            ui.set_status_kind(StatusKind::Success);
                            ui.set_status_message("Saved.".into());
                            // Refresh the link index for this note only.
                            let refresh = make_refresh(ui.as_weak(), Arc::clone(&shared3));
                            worker::reindex_note(id, Arc::clone(&shared3), refresh);
                        }
                        Err(e) => {
                            ui.set_status_kind(StatusKind::Error);
                            ui.set_status_message(format!("Save error: {e}").into());
                        }
                    }
                },
            );
        });
    }

    // ── save-requested ────────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_save_requested(move || {
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            let id = ui.get_current_note_id().to_string();
            let content = ui.get_current_note_content().to_string();
            if id.is_empty() {
                return;
            }
            let vault = {
                let data = shared2.lock().unwrap();
                data.vault_path.clone()
            };
            match vault::write_note(&vault, &id, &content) {
                Ok(()) => {
                    ui.set_unsaved(false);
                    mark_tab_unsaved(&ui, &id, false);
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message("Saved.".into());
                    let refresh = make_refresh(ui_weak.clone(), Arc::clone(&shared2));
                    worker::reindex_note(id, Arc::clone(&shared2), refresh);
                }
                Err(e) => {
                    ui.set_status_kind(StatusKind::Error);
                    ui.set_status_message(format!("Save error: {e}").into());
                }
            }
        });
    }

    // ── new-note-requested (opens the create dialog) ──────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_new_note_requested(move || {
            let (vault, folder) = {
                let data = shared2.lock().unwrap();
                (data.vault_path.clone(), data.selected_folder.clone())
            };
            if vault.as_os_str().is_empty() {
                return; // No vault open yet
            }
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            open_create_dialog(&ui, &vault, &folder, CreateKind::Note);
        });
    }

    // ── new-note-in-folder (row context menu) ─────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_new_note_in_folder(move |folder| {
            let folder_str = folder.to_string();
            let vault = {
                let data = shared2.lock().unwrap();
                data.vault_path.clone()
            };
            if vault.as_os_str().is_empty() {
                return;
            }
            if let Ok(mut data) = shared2.lock() {
                data.selected_folder = folder_str.clone();
            }
            if let Some(ui) = ui_weak.upgrade() {
                open_create_dialog(&ui, &vault, &folder_str, CreateKind::Note);
                if let Ok(data) = shared2.lock() {
                    refresh_sidebar(&ui, &data);
                }
            }
        });
    }

    // ── create-note-confirmed (name submitted from the dialog) ────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_create_note_confirmed(move |name| {
            let title = name.trim().to_owned();
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            if title.is_empty() {
                ui.set_create_error("Please enter a name.".into());
                return;
            }
            if title.contains('/') || title.contains('\\') || title == "." || title == ".." {
                ui.set_create_error("Names cannot contain / or \\.".into());
                return;
            }
            let (vault, folder) = {
                let data = shared2.lock().unwrap();
                (data.vault_path.clone(), data.selected_folder.clone())
            };
            if vault.as_os_str().is_empty() {
                return;
            }
            match vault::create_note_in(&vault, &folder, &title) {
                Ok(new_id) => {
                    ui.set_show_create_dialog(false);
                    ui.set_create_error("".into());
                    ui.set_current_note_id(new_id.clone().into());
                    ui.set_current_note_title(title.clone().into());
                    ui.set_current_note_content("".into());
                    render_preview(&ui, "");
                    ui.set_unsaved(false);
                    ui.set_backlinks(ModelRc::new(VecModel::from(vec![])));
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message(format!("Created: {title}").into());
                    add_or_activate_tab(&ui, &new_id, &title);
                    persist_session(&ui, &shared2);
                    let refresh = make_refresh(ui_weak.clone(), Arc::clone(&shared2));
                    worker::rebuild_index(vault, Arc::clone(&shared2), refresh);
                }
                Err(e) => {
                    ui.set_status_kind(StatusKind::Error);
                    ui.set_status_message(format!("Error creating note: {e}").into());
                    ui.set_create_error(format!("{e}").into());
                }
            }
        });
    }

    // ── delete-requested ──────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_delete_requested(move |id| {
            let id_str = id.to_string();
            let vault = {
                let data = shared2.lock().unwrap();
                data.vault_path.clone()
            };
            match vault::delete_note(&vault, &id_str) {
                Ok(()) => {
                    let ui = match ui_weak.upgrade() {
                        Some(u) => u,
                        None => return,
                    };
                    remove_tab(&ui, &id_str);
                    if ui.get_current_note_id().as_str() == id_str {
                        ui.set_current_note_id("".into());
                        ui.set_current_note_title("".into());
                        ui.set_current_note_content("".into());
                        render_preview(&ui, "");
                        ui.set_backlinks(ModelRc::new(VecModel::from(vec![])));
                    }
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message("Note deleted.".into());
                    persist_session(&ui, &shared2);
                    let refresh = make_refresh(ui_weak.clone(), Arc::clone(&shared2));
                    worker::rebuild_index(vault, Arc::clone(&shared2), refresh);
                }
                Err(e) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_status_kind(StatusKind::Error);
                        ui.set_status_message(format!("Delete error: {e}").into());
                    }
                }
            }
        });
    }

    // ── rename-requested ──────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_rename_requested(move |id, new_title| {
            let id_str = id.to_string();
            let new_title_str = new_title.to_string();
            let vault = {
                let data = shared2.lock().unwrap();
                data.vault_path.clone()
            };
            match vault::rename_note(&vault, &id_str, &new_title_str) {
                Ok(new_id) => {
                    let ui = match ui_weak.upgrade() {
                        Some(u) => u,
                        None => return,
                    };
                    if ui.get_current_note_id().as_str() == id_str {
                        ui.set_current_note_id(new_id.clone().into());
                        ui.set_current_note_title(new_title_str.clone().into());
                    }
                    rename_tab(&ui, &id_str, &new_id, &new_title_str);
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message(format!("Renamed to \"{new_title_str}\"").into());
                    persist_session(&ui, &shared2);
                    let refresh = make_refresh(ui_weak.clone(), Arc::clone(&shared2));
                    worker::rebuild_index(vault, Arc::clone(&shared2), refresh);
                }
                Err(e) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_status_kind(StatusKind::Error);
                        ui.set_status_message(format!("Rename error: {e}").into());
                    }
                }
            }
        });
    }

    // ── note-moved (drag & drop onto a folder) ────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_note_moved(move |id, folder| {
            let id_str = id.to_string();
            let folder_str = folder.to_string();
            let vault = {
                let data = shared2.lock().unwrap();
                data.vault_path.clone()
            };
            if vault.as_os_str().is_empty() {
                return;
            }
            match vault::move_note(&vault, &id_str, &folder_str) {
                Ok(new_id) => {
                    let ui = match ui_weak.upgrade() {
                        Some(u) => u,
                        None => return,
                    };
                    if new_id == id_str {
                        ui.set_status_kind(StatusKind::Info);
                        ui.set_status_message("Note is already in that folder.".into());
                        return;
                    }
                    // The note id encodes its path, so moving changes it: keep
                    // the active note and any open tab pointing at the new path.
                    if ui.get_current_note_id().as_str() == id_str {
                        ui.set_current_note_id(new_id.clone().into());
                    }
                    move_tab(&ui, &id_str, &new_id);
                    if let Ok(mut data) = shared2.lock() {
                        data.selected_folder = folder_str.clone();
                    }
                    let destination = if folder_str.trim().is_empty() {
                        "vault root".to_owned()
                    } else {
                        folder_str.clone()
                    };
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message(format!("Moved to {destination}").into());
                    persist_session(&ui, &shared2);
                    let refresh = make_refresh(ui_weak.clone(), Arc::clone(&shared2));
                    worker::rebuild_index(vault, Arc::clone(&shared2), refresh);
                }
                Err(e) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_status_kind(StatusKind::Error);
                        ui.set_status_message(format!("Move error: {e}").into());
                    }
                }
            }
        });
    }

    // ── new-folder-requested (opens the create dialog) ────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_new_folder_requested(move || {
            let (vault, parent) = {
                let data = shared2.lock().unwrap();
                (data.vault_path.clone(), data.selected_folder.clone())
            };
            if vault.as_os_str().is_empty() {
                return;
            }
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            open_create_dialog(&ui, &vault, &parent, CreateKind::Folder);
        });
    }

    // ── new-folder-in-folder (row context menu) ───────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_new_folder_in_folder(move |folder| {
            let folder_str = folder.to_string();
            let vault = {
                let data = shared2.lock().unwrap();
                data.vault_path.clone()
            };
            if vault.as_os_str().is_empty() {
                return;
            }
            if let Ok(mut data) = shared2.lock() {
                data.selected_folder = folder_str.clone();
            }
            if let Some(ui) = ui_weak.upgrade() {
                open_create_dialog(&ui, &vault, &folder_str, CreateKind::Folder);
                if let Ok(data) = shared2.lock() {
                    refresh_sidebar(&ui, &data);
                }
            }
        });
    }

    // ── create-folder-confirmed (name submitted from the dialog) ──────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_create_folder_confirmed(move |name| {
            let name = name.trim().to_owned();
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            if name.is_empty() {
                ui.set_create_error("Please enter a name.".into());
                return;
            }
            let (vault, parent) = {
                let data = shared2.lock().unwrap();
                (data.vault_path.clone(), data.selected_folder.clone())
            };
            if vault.as_os_str().is_empty() {
                return;
            }
            match vault::create_folder(&vault, &parent, &name) {
                Ok(path) => {
                    ui.set_show_create_dialog(false);
                    ui.set_create_error("".into());
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message(format!("Created folder: {path}").into());
                    let refresh = make_refresh(ui_weak.clone(), Arc::clone(&shared2));
                    worker::rebuild_index(vault, Arc::clone(&shared2), refresh);
                }
                Err(e) => {
                    ui.set_status_kind(StatusKind::Error);
                    ui.set_status_message(format!("Error creating folder: {e}").into());
                    ui.set_create_error(format!("{e}").into());
                }
            }
        });
    }

    // ── rename-folder-requested ───────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_rename_folder_requested(move |path, new_name| {
            let path_str = path.to_string();
            let new_name_str = new_name.to_string();
            let vault = {
                let data = shared2.lock().unwrap();
                data.vault_path.clone()
            };
            match vault::rename_folder(&vault, &path_str, &new_name_str) {
                Ok(new_path) => {
                    let ui = match ui_weak.upgrade() {
                        Some(u) => u,
                        None => return,
                    };
                    // Notes inside the renamed folder change id, so fix the
                    // open tabs, the active note and the stored UI state.
                    remap_tabs(&ui, &path_str, &new_path);
                    if let Ok(mut data) = shared2.lock() {
                        remap_folder_state(&mut data, &path_str, &new_path);
                    }
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message(format!("Renamed folder to \"{new_path}\"").into());
                    persist_session(&ui, &shared2);
                    let refresh = make_refresh(ui_weak.clone(), Arc::clone(&shared2));
                    worker::rebuild_index(vault, Arc::clone(&shared2), refresh);
                }
                Err(e) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_status_kind(StatusKind::Error);
                        ui.set_status_message(format!("Rename folder error: {e}").into());
                    }
                }
            }
        });
    }

    // ── delete-folder-requested ───────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_delete_folder_requested(move |path| {
            let path_str = path.to_string();
            let vault = {
                let data = shared2.lock().unwrap();
                data.vault_path.clone()
            };
            match vault::delete_folder(&vault, &path_str) {
                Ok(()) => {
                    let ui = match ui_weak.upgrade() {
                        Some(u) => u,
                        None => return,
                    };
                    drop_tabs_in_folder(&ui, &path_str);
                    if let Ok(mut data) = shared2.lock() {
                        drop_folder_state(&mut data, &path_str);
                    }
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message(format!("Deleted folder: {path_str}").into());
                    persist_session(&ui, &shared2);
                    let refresh = make_refresh(ui_weak.clone(), Arc::clone(&shared2));
                    worker::rebuild_index(vault, Arc::clone(&shared2), refresh);
                }
                Err(e) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_status_kind(StatusKind::Error);
                        ui.set_status_message(format!("Delete folder error: {e}").into());
                    }
                }
            }
        });
    }

    // ── toggle-sort-requested ─────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_toggle_sort_requested(move || {
            if let Ok(mut data) = shared2.lock() {
                data.sort_descending = !data.sort_descending;
            }
            if let Some(ui) = ui_weak.upgrade() {
                if let Ok(data) = shared2.lock() {
                    refresh_sidebar(&ui, &data);
                }
            }
        });
    }

    // ── toggle-collapse-folders-requested (expand/collapse all) ───────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_toggle_collapse_folders_requested(move || {
            if let Ok(mut data) = shared2.lock() {
                let collapse = !all_folders_collapsed(&data);
                let mut paths = Vec::new();
                collect_folder_paths(&data.tree, &mut paths);
                for path in paths {
                    data.expanded.insert(path, !collapse);
                }
            }
            if let Some(ui) = ui_weak.upgrade() {
                if let Ok(data) = shared2.lock() {
                    refresh_sidebar(&ui, &data);
                }
            }
        });
    }

    // ── toggle-sidebar-requested (collapse / expand the sidebar) ──────────────
    {
        let ui_weak = ui.as_weak();
        ui.on_toggle_sidebar_requested(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let collapsed = ui.get_sidebar_collapsed();
                ui.set_sidebar_collapsed(!collapsed);
            }
        });
    }

    // ── tab-closed ────────────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_tab_closed(move |id| {
            let id_str = id.to_string();
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };

            {
                let data = shared2.lock().unwrap();
                if ui.get_current_note_id().as_str() == id_str && ui.get_unsaved() {
                    let content = ui.get_current_note_content().to_string();
                    let _ = vault::write_note(&data.vault_path, &id_str, &content);
                }
            }

            let was_active = ui.get_current_note_id().as_str() == id_str;
            remove_tab(&ui, &id_str);

            if was_active {
                let tabs_model = ui.get_open_tabs();
                let tabs_len = tabs_model.row_count();
                if tabs_len > 0 {
                    let next = tabs_model.row_data(tabs_len - 1).unwrap();
                    let next_id = next.id.to_string();
                    let data = shared2.lock().unwrap();
                    if let Ok(note) = vault::read_note(&data.vault_path, &next_id) {
                        ui.set_current_note_id(note.meta.id.clone().into());
                        ui.set_current_note_title(note.meta.title.clone().into());
                        ui.set_current_note_content(note.content.clone().into());
                        render_preview(&ui, &note.content);
                        ui.set_unsaved(false);
                    }
                } else {
                    ui.set_current_note_id("".into());
                    ui.set_current_note_title("".into());
                    ui.set_current_note_content("".into());
                    render_preview(&ui, "");
                    ui.set_unsaved(false);
                    ui.set_backlinks(ModelRc::new(VecModel::from(vec![])));
                }
            }
            persist_session(&ui, &shared2);
        });
    }

    // ── search-changed (debounced 150 ms) ────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        let search_timer = Rc::new(Timer::default());
        ui.on_search_changed(move |query| {
            let query_str = query.to_string();
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };

            if query_str.trim().is_empty() {
                ui.set_search_results(ModelRc::new(VecModel::from(vec![])));
                return;
            }

            let vault = {
                let data = shared2.lock().unwrap();
                data.vault_path.clone()
            };
            if vault.as_os_str().is_empty() {
                return;
            }
            let ui_weak2 = ui_weak.clone();
            search_timer.start(
                TimerMode::SingleShot,
                std::time::Duration::from_millis(150),
                move || {
                    worker::run_search(query_str.clone(), vault.clone(), {
                        let ui_weak3 = ui_weak2.clone();
                        move |results| {
                            let ui = match ui_weak3.upgrade() {
                                Some(u) => u,
                                None => return,
                            };
                            let items: Vec<NoteItem> = results
                                .into_iter()
                                .map(|r| NoteItem {
                                    folder: {
                                        let p = std::path::Path::new(&r.meta.id);
                                        match p.parent() {
                                            Some(par) if par.as_os_str() != "" => {
                                                par.to_string_lossy().into_owned()
                                            }
                                            _ => String::new(),
                                        }
                                    }
                                    .into(),
                                    id: r.meta.id.into(),
                                    title: r.meta.title.into(),
                                })
                                .collect();
                            ui.set_search_results(ModelRc::new(VecModel::from(items)));
                        }
                    });
                },
            );
        });
    }

    // ── toggle-preview (Ctrl+P) — cycles through view modes ──────────────────
    {
        let ui_weak = ui.as_weak();
        ui.on_toggle_preview(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let next = match ui.get_view_mode() {
                    ViewMode::Edit => ViewMode::Split,
                    ViewMode::Split => ViewMode::Preview,
                    ViewMode::Preview => ViewMode::Edit,
                };
                ui.set_view_mode(next);
            }
        });
    }

    // ── toggle-theme (button / Ctrl+T) ────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        ui.on_toggle_theme(move || {
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            let dark = !ui.get_dark_mode();
            ui.set_dark_mode(dark);
            apply_theme(&ui, dark);

            let mut config = Config::load().unwrap_or(Config {
                vault_path: PathBuf::new(),
                dark_mode: dark,
                open_tabs: Vec::new(),
                active_note: String::new(),
                check_updates: false,
                skipped_version: String::new(),
                attachment_folder: "attachments".into(),
            });
            config.dark_mode = dark;
            let _ = config.save();
        });
    }

    // ── Markdown editing assistance ───────────────────────────────────────────
    {
        let shared2 = Arc::clone(&shared);
        ui.on_handle_edit_key(move |key, _ctrl, _shift, _alt, content, cursor, anchor| {
            let titles: Vec<String> = shared2
                .lock()
                .unwrap()
                .notes
                .iter()
                .map(|n| n.title.clone())
                .collect();
            editor_assist::handle_key(key.as_str(), content.as_str(), cursor, anchor, &titles)
        });
    }
    {
        ui.on_accept_suggestion(|title, content, cursor| {
            editor_assist::accept_suggestion(title.as_str(), content.as_str(), cursor)
        });
    }
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_update_suggestions(move |content, cursor| {
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            let titles: Vec<String> = shared2
                .lock()
                .unwrap()
                .notes
                .iter()
                .map(|n| n.title.clone())
                .collect();
            let (suggestions, show) =
                editor_assist::suggestions(content.as_str(), cursor.max(0) as usize, &titles);
            let suggestions: Vec<SharedString> =
                suggestions.into_iter().map(SharedString::from).collect();
            ui.set_suggestions(ModelRc::new(VecModel::from(suggestions)));
            ui.set_show_suggestions(show);
        });
    }

    // ── Flush pending changes before the window closes ────────────────────────
    //
    // The autosave timer is a debounce, so a quick type-then-close could lose
    // the last keystrokes.  Persist the active note synchronously first, then
    // let the window hide (returning `HideWindow` is the default action).
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.window().on_close_requested(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let id = ui.get_current_note_id().to_string();
                let content = ui.get_current_note_content().to_string();
                if !id.is_empty() && ui.get_unsaved() {
                    let data = shared2.lock().unwrap();
                    if let Err(e) = vault::write_note(&data.vault_path, &id, &content) {
                        eprintln!("[rustidian] error saving on close: {e}");
                    }
                }
                // Remember the tab set and active note for next launch.
                persist_session(&ui, &shared2);
            }
            slint::CloseRequestResponse::HideWindow
        });
    }

    // ── Update flow (manual, con chequeo opcional al inicio) ──────────────────
    {
        let ui_weak = ui.as_weak();
        ui.on_check_updates_requested(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            ui.set_show_update_dialog(true);
            ui.set_update_checking(true);
            ui.set_update_available(false);
            ui.set_update_message("Buscando actualizaciones…".into());

            let ui_weak2 = ui.as_weak();
            std::thread::spawn(move || {
                let result = update::check();
                slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak2.upgrade() {
                        ui.set_update_checking(false);
                        match result {
                            Ok(Some(info)) => {
                                ui.set_update_available(true);
                                ui.set_update_latest(info.version.clone().into());
                                ui.set_update_message(
                                    format!(
                                        "Hay una versión nueva: {} (tienes {}).",
                                        info.version,
                                        env!("CARGO_PKG_VERSION")
                                    )
                                    .into(),
                                );
                            }
                            Ok(None) => {
                                ui.set_update_message(
                                    format!(
                                        "Ya tienes la última versión ({}).",
                                        env!("CARGO_PKG_VERSION")
                                    )
                                    .into(),
                                );
                            }
                            Err(e) => {
                                ui.set_update_message(
                                    format!("No se pudieron comprobar: {e}").into(),
                                );
                            }
                        }
                    }
                })
                .ok();
            });
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_update_install_requested(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            ui.set_update_checking(true);
            ui.set_update_available(false);
            ui.set_update_message("Descargando actualización…".into());

            let ui_weak2 = ui.as_weak();
            std::thread::spawn(move || {
                let result = update::check().and_then(|maybe| match maybe {
                    Some(info) => update::download_and_apply(&info).map(|()| info.version),
                    None => Err("Ya tienes la última versión.".to_string()),
                });
                slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak2.upgrade() {
                        ui.set_update_checking(false);
                        match result {
                            Ok(version) => ui.set_update_message(
                                format!("Actualizado a {version}. Reinicia Rustidian para usarla.")
                                    .into(),
                            ),
                            Err(e) => {
                                ui.set_update_message(format!("Error al actualizar: {e}").into())
                            }
                        }
                    }
                })
                .ok();
            });
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_update_dismissed(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_show_update_dialog(false);
            }
        });
    }

    if check_updates_on_start {
        let ui_weak = ui.as_weak();
        std::thread::spawn(move || {
            if let Ok(Some(info)) = update::check() {
                slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_update_available(true);
                        ui.set_update_latest(info.version.clone().into());
                        ui.set_update_message(
                            format!("Hay una versión nueva: {}.", info.version).into(),
                        );
                        ui.set_show_update_dialog(true);
                    }
                })
                .ok();
            }
        });
    }

    // ── Imágenes: insertar, ver en grande y guardar ───────────────────────────
    {
        // Splice the generated Markdown at the caret inside the editor.
        ui.on_request_insert(|insert, content, cursor| {
            editor_assist::insert_at(content.as_str(), cursor, insert.as_str())
        });
    }

    // ── Formatting shortcuts (Ctrl+B, Ctrl+I, Ctrl+Shift+K) ──────────────────
    {
        ui.on_format_selection(|kind, content, cursor, anchor| {
            editor_assist::format_selection(content.as_str(), cursor, anchor, kind.as_str())
        });
    }

    // ── Paste image from clipboard (Ctrl+V) ──────────────────────────────────
    {
        let shared2 = Arc::clone(&shared);
        let ui_weak = ui.as_weak();
        ui.on_paste_image_requested(move |content, cursor| {
            let mut result = EditResult {
                handled: false,
                text: content.clone(),
                cursor,
                anchor: cursor,
            };

            // Try to get image from clipboard using arboard
            let clipboard_result = Clipboard::new().and_then(|mut clipboard| clipboard.get_image());
            if let Ok(image_data) = clipboard_result {
                let vault = shared2.lock().unwrap().vault_path.clone();
                let (note_id, attachments) = {
                    let ui = match ui_weak.upgrade() {
                        Some(u) => u,
                        None => return result,
                    };
                    (
                        ui.get_current_note_id().to_string(),
                        ui.get_attachment_folder().to_string(),
                    )
                };

                // Generate a unique filename
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let filename = format!("pasted-{}.png", timestamp);

                // Build attachment directory path
                let vault_path = PathBuf::from(&vault);
                let attach_dir = vault_path.join(&attachments);
                let _ = std::fs::create_dir_all(&attach_dir);

                // Save the image as PNG
                let img_path = attach_dir.join(&filename);
                let img = image::RgbaImage::from_raw(
                    image_data.width as u32,
                    image_data.height as u32,
                    image_data.bytes.to_vec(),
                );

                if let Some(img) = img {
                    if img.save(&img_path).is_ok() {
                        // Build relative path for markdown
                        let note_folder = vault::note_dir(&vault_path, &note_id);
                        let rel_path =
                            pathdiff::diff_paths(&img_path, &note_folder).unwrap_or(img_path);
                        let rel = rel_path.to_string_lossy().replace('\\', "/");
                        let mark = format!("\n![pasted-image]({rel})\n");

                        // Clamp cursor and insert
                        let cur = {
                            let mut o = (cursor.max(0) as usize).min(content.len());
                            while o > 0 && !content.is_char_boundary(o) {
                                o -= 1;
                            }
                            o
                        };
                        let mut out = String::with_capacity(content.len() + mark.len());
                        out.push_str(&content[..cur]);
                        out.push_str(&mark);
                        out.push_str(&content[cur..]);
                        result = EditResult {
                            handled: true,
                            text: out.into(),
                            cursor: (cur + mark.len()) as i32,
                            anchor: (cur + mark.len()) as i32,
                        };
                    }
                }
            }

            result
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_image_clicked(move |path| {
            if let Some(ui) = ui_weak.upgrade() {
                let p = PathBuf::from(path.as_str());
                match bridge::load_full_image(&p) {
                    Some(image) => {
                        let name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.to_string());
                        ui.set_modal_image(image);
                        ui.set_modal_image_name(name.into());
                        ui.set_modal_image_path(path.clone());
                        ui.set_show_image_modal(true);
                    }
                    None => {
                        ui.set_status_kind(StatusKind::Error);
                        ui.set_status_message("No se pudo abrir la imagen.".into());
                    }
                }
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_image_modal_closed(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_show_image_modal(false);
                ui.set_modal_image(slint::Image::default());
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_save_image_requested(move |path| {
            let ui_weak = ui_weak.clone();
            let src = PathBuf::from(path.as_str());
            std::thread::spawn(move || {
                let default_name = src
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "image.png".to_owned());
                if let Some(dest) = rfd::FileDialog::new()
                    .set_file_name(default_name)
                    .save_file()
                {
                    let result = std::fs::copy(&src, &dest);
                    if let Some(ui) = ui_weak.upgrade() {
                        match result {
                            Ok(_) => {
                                ui.set_status_kind(StatusKind::Success);
                                ui.set_status_message(
                                    format!("Imagen guardada en {}", dest.display()).into(),
                                );
                            }
                            Err(e) => {
                                ui.set_status_kind(StatusKind::Error);
                                ui.set_status_message(format!("No se pudo guardar: {e}").into());
                            }
                        }
                    }
                }
            });
        });
    }

    // Insert an image chosen with the native file picker.
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_insert_image_requested(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let vault = shared2.lock().unwrap().vault_path.clone();
            let note_id = ui.get_current_note_id().to_string();
            let attachments = ui.get_attachment_folder().to_string();
            let ui_weak = ui.as_weak();
            std::thread::spawn(move || {
                let picked = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "svg"])
                    .pick_file();
                let Some(src) = picked else { return };
                match vault::import_attachment(&vault, &note_id, &src, &attachments) {
                    Ok(rel) => {
                        let alt = src
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let markdown = image_markdown(&alt, &rel);
                        slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_editor_insert(markdown.into());
                            }
                        })
                        .ok();
                    }
                    Err(e) => {
                        slint::invoke_from_event_loop(move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_status_kind(StatusKind::Error);
                                ui.set_status_message(
                                    format!("No se pudo importar la imagen: {e}").into(),
                                );
                            }
                        })
                        .ok();
                    }
                }
            });
        });
    }

    // Insert images dropped onto the editor.
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_files_dropped(move |data| {
            let paths: Vec<PathBuf> = data
                .file_paths()
                .map(|it| it.map(|p| p.to_path_buf()).collect())
                .unwrap_or_default();
            if paths.is_empty() {
                return;
            }
            let Some(ui) = ui_weak.upgrade() else { return };
            let vault = shared2.lock().unwrap().vault_path.clone();
            let note_id = ui.get_current_note_id().to_string();
            let attachments = ui.get_attachment_folder().to_string();
            let mut marks = Vec::new();
            for src in paths {
                if let Ok(rel) = vault::import_attachment(&vault, &note_id, &src, &attachments) {
                    let alt = src
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    marks.push(image_markdown(&alt, &rel));
                }
            }
            if !marks.is_empty() {
                ui.set_editor_insert(marks.join("\n").into());
            }
        });
    }

    ui.run()?;
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Convert a `#rrggbb` integer to a Slint colour.
fn rgb(hex: u32) -> Color {
    Color::from_rgb_u8(
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

/// Apply the Catppuccin palette (Mocha for dark, Latte for light) to the UI.
fn apply_theme(ui: &AppWindow, dark: bool) {
    let p = ui.global::<Palette>();
    if dark {
        // Catppuccin Mocha
        p.set_bg(rgb(0x1e1e2e));
        p.set_surface(rgb(0x181825));
        p.set_card(rgb(0x313244));
        p.set_border(rgb(0x45475a));
        p.set_text(rgb(0xcdd6f4));
        p.set_text_muted(rgb(0xa6adc8));
        p.set_accent(rgb(0x74c7ec));
        p.set_on_accent(rgb(0x1e1e2e));
        p.set_success(rgb(0xa6e3a1));
        p.set_danger(rgb(0xf38ba8));
        p.set_warning(rgb(0xf9e2af));
    } else {
        // Catppuccin Latte
        p.set_bg(rgb(0xeff1f5));
        p.set_surface(rgb(0xe6e9ef));
        p.set_card(rgb(0xccd0da));
        p.set_border(rgb(0xbcc0cc));
        p.set_text(rgb(0x4c4f69));
        p.set_text_muted(rgb(0x6c6f85));
        p.set_accent(rgb(0x209fb5));
        p.set_on_accent(rgb(0x1e1e2e));
        p.set_success(rgb(0x40a02b));
        p.set_danger(rgb(0xd20f39));
        p.set_warning(rgb(0xdf8e1d));
    }
}

/// Render Markdown content into the preview block model.
fn render_preview(ui: &AppWindow, content: &str) {
    let note_id = ui.get_current_note_id().to_string();
    let vault = PathBuf::from(ui.get_vault_path().as_str());
    let attachments = ui.get_attachment_folder().to_string();
    let items = blocks_to_items(&parse_blocks(content), &vault, &note_id, &attachments);
    ui.set_preview_blocks(ModelRc::new(VecModel::from(items)));
}

/// Markdown to insert for an image, placed on its own line.
fn image_markdown(alt: &str, rel: &str) -> String {
    let alt = alt.replace('[', "(").replace(']', ")");
    format!("\n![{alt}]({rel})\n")
}

/// Rebuild the sidebar tree, note titles and backlinks from shared state.
fn refresh_sidebar(ui: &AppWindow, data: &AppData) {
    let rows = tree_to_rows(
        &data.tree,
        &data.expanded,
        &data.selected_folder,
        data.sort_descending,
    );
    ui.set_note_count(data.notes.len() as i32);
    ui.set_tree(ModelRc::new(VecModel::from(rows)));
    ui.set_sort_descending(data.sort_descending);
    ui.set_all_folders_collapsed(all_folders_collapsed(data));

    let titles: Vec<SharedString> = data.notes.iter().map(|n| n.title.clone().into()).collect();
    ui.set_note_titles(ModelRc::new(VecModel::from(titles)));

    let current_id = ui.get_current_note_id().to_string();
    if !current_id.is_empty() {
        let bl_ids = data
            .link_index
            .backlinks
            .get(&normalise(&current_id))
            .cloned()
            .unwrap_or_default();
        let bl_items = to_backlink_items(&bl_ids, &data.notes);
        ui.set_backlinks(ModelRc::new(VecModel::from(bl_items)));
    }
}

/// Collect every folder path in the tree (depth-first, excluding the root).
fn collect_folder_paths(nodes: &[vault::FolderNode], out: &mut Vec<String>) {
    for node in nodes {
        if !node.path.is_empty() {
            out.push(node.path.clone());
        }
        collect_folder_paths(&node.children, out);
    }
}

/// Whether the tree has folders and every one of them is collapsed.
///
/// Folders missing from the `expanded` map are collapsed by default, so a
/// missing entry counts as collapsed here too.
fn all_folders_collapsed(data: &AppData) -> bool {
    let mut paths = Vec::new();
    collect_folder_paths(&data.tree, &mut paths);
    !paths.is_empty()
        && paths
            .iter()
            .all(|p| !data.expanded.get(p).copied().unwrap_or(false))
}

/// Build a `Send` refresh callback for background workers.
fn make_refresh(
    ui_weak: slint::Weak<AppWindow>,
    shared: Arc<Mutex<AppData>>,
) -> impl Fn() + Send + 'static {
    move || {
        if let Some(ui) = ui_weak.upgrade() {
            let data = shared.lock().unwrap();
            refresh_sidebar(&ui, &data);
        }
    }
}

/// Restore the open tabs and active note saved in the config.
///
/// Tabs whose files no longer exist are dropped.  The previously active note
/// is opened; if it is gone, the last surviving tab is used instead.
fn restore_session(ui: &AppWindow, vault: &std::path::Path, tabs: &[String], active: &str) {
    let items: Vec<TabItem> = tabs
        .iter()
        .filter_map(|id| vault::read_note(vault, id).ok())
        .map(|note| TabItem {
            id: note.meta.id.into(),
            title: note.meta.title.into(),
            unsaved: false,
        })
        .collect();
    let count = items.len();
    ui.set_open_tabs(ModelRc::new(VecModel::from(items)));

    // Try the previously active note first, then the surviving tabs last-to-first.
    let candidates = std::iter::once(active.to_owned()).chain(
        (0..count)
            .rev()
            .filter_map(|i| ui.get_open_tabs().row_data(i))
            .map(|t| t.id.to_string()),
    );
    for id in candidates {
        if id.is_empty() {
            continue;
        }
        if let Ok(note) = vault::read_note(vault, &id) {
            ui.set_current_note_id(note.meta.id.clone().into());
            ui.set_current_note_title(note.meta.title.clone().into());
            ui.set_current_note_content(note.content.clone().into());
            render_preview(ui, &note.content);
            ui.set_unsaved(false);
            ui.set_status_kind(StatusKind::Success);
            ui.set_status_message(format!("Opened: {}", note.meta.title).into());
            break;
        }
    }
}

/// Persist the open tabs and active note to the config file.
fn persist_session(ui: &AppWindow, shared: &Arc<Mutex<AppData>>) {
    let vault = {
        let data = shared.lock().unwrap();
        data.vault_path.clone()
    };
    if vault.as_os_str().is_empty() {
        return;
    }

    let tabs_model = ui.get_open_tabs();
    let tabs: Vec<String> = (0..tabs_model.row_count())
        .filter_map(|i| tabs_model.row_data(i))
        .map(|t| t.id.to_string())
        .collect();

    let mut config = Config::load().unwrap_or(Config {
        vault_path: vault.clone(),
        dark_mode: ui.get_dark_mode(),
        open_tabs: Vec::new(),
        active_note: String::new(),
        check_updates: false,
        skipped_version: String::new(),
        attachment_folder: "attachments".into(),
    });
    config.vault_path = vault;
    config.dark_mode = ui.get_dark_mode();
    config.open_tabs = tabs;
    config.active_note = ui.get_current_note_id().to_string();
    let _ = config.save();
}

/// Resolve a wikilink target to a note id.
///
/// Matching is case-insensitive: first by relative path (adding `.md` when
/// missing), then by note title.
fn resolve_wikilink(notes: &[rustidian_core::vault::NoteMeta], target: &str) -> Option<String> {
    let target_lower = target.to_lowercase();
    let with_ext = if target_lower.ends_with(".md") {
        target_lower.clone()
    } else {
        format!("{target_lower}.md")
    };
    if let Some(note) = notes.iter().find(|n| n.id.to_lowercase() == with_ext) {
        return Some(note.id.clone());
    }
    notes
        .iter()
        .find(|n| n.title.to_lowercase() == target_lower)
        .map(|n| n.id.clone())
}

/// Set the vault path in shared state and update the UI status bar.
fn open_vault(vault: PathBuf, shared: Arc<Mutex<AppData>>, ui_weak: slint::Weak<AppWindow>) {
    {
        let mut data = shared.lock().unwrap();
        data.vault_path = vault.clone();
    }
    if let Some(ui) = ui_weak.upgrade() {
        ui.set_vault_path(vault.to_string_lossy().into_owned().into());
        if let Ok(cfg) = Config::load() {
            ui.set_attachment_folder(cfg.attachment_folder.into());
        }
        ui.set_status_kind(StatusKind::Info);
        ui.set_status_message(format!("Vault: {}", vault.display()).into());
    }
}

/// Persist the chosen vault, switch the UI to the main layout and load it.
fn activate_vault(path: PathBuf, shared: Arc<Mutex<AppData>>, ui_weak: slint::Weak<AppWindow>) {
    let mut config = Config::load().unwrap_or(Config {
        vault_path: path.clone(),
        dark_mode: true,
        open_tabs: Vec::new(),
        active_note: String::new(),
        check_updates: false,
        skipped_version: String::new(),
        attachment_folder: "attachments".into(),
    });
    config.vault_path = path.clone();
    // A different vault starts with a fresh session.
    config.open_tabs = Vec::new();
    config.active_note = String::new();
    let _ = config.save();

    if let Some(ui) = ui_weak.upgrade() {
        ui.set_vault_picker_error("".into());
        ui.set_show_vault_picker(false);
        ui.set_status_kind(StatusKind::Info);
        ui.set_status_message(format!("Vault: {}", path.display()).into());
        ui.set_open_tabs(ModelRc::new(VecModel::from(vec![])));
    }

    open_vault(path.clone(), Arc::clone(&shared), ui_weak.clone());
    let refresh = make_refresh(ui_weak, Arc::clone(&shared));
    worker::rebuild_index(path, shared, refresh);
}

/// Open the create dialog pre-filled with a non-colliding default name.
///
/// `kind` selects whether a note or a folder is being named; the actual
/// creation happens in the `create-*-confirmed` handlers once the user submits.
fn open_create_dialog(ui: &AppWindow, vault: &std::path::Path, folder: &str, kind: CreateKind) {
    let name = match kind {
        CreateKind::Note => next_untitled_name(vault, folder),
        CreateKind::Folder => next_folder_name(vault, folder),
    };
    ui.set_create_kind(kind);
    ui.set_create_name(name.into());
    ui.set_create_error("".into());
    ui.set_show_create_dialog(true);
}

/// Return an "Untitled N" name that does not collide inside *folder*.
fn next_untitled_name(vault: &std::path::Path, folder: &str) -> String {
    let dir = if folder.trim().is_empty() {
        vault.to_path_buf()
    } else {
        vault.join(folder.trim())
    };
    let mut n = 1u32;
    loop {
        let name = if n == 1 {
            "Untitled".to_owned()
        } else {
            format!("Untitled {n}")
        };
        if !dir.join(format!("{name}.md")).exists() {
            return name;
        }
        n += 1;
    }
}

/// Return a "New folder N" name that does not collide inside *parent*.
fn next_folder_name(vault: &std::path::Path, parent: &str) -> String {
    let dir = if parent.trim().is_empty() {
        vault.to_path_buf()
    } else {
        vault.join(parent.trim())
    };
    let mut n = 1u32;
    loop {
        let name = if n == 1 {
            "New folder".to_owned()
        } else {
            format!("New folder {n}")
        };
        if !dir.join(&name).exists() {
            return name;
        }
        n += 1;
    }
}

/// Remap folder expansion/selection state after a folder rename.
fn remap_folder_state(data: &mut AppData, old: &str, new: &str) {
    let prefix = format!("{old}/");
    if data.selected_folder == old {
        data.selected_folder = new.to_owned();
    } else if let Some(rest) = data.selected_folder.strip_prefix(&prefix) {
        data.selected_folder = format!("{new}/{rest}");
    }

    let mut moved: Vec<(String, bool)> = Vec::new();
    data.expanded.retain(|key, value| {
        if key == old {
            moved.push((new.to_owned(), *value));
            false
        } else if let Some(rest) = key.strip_prefix(&prefix) {
            moved.push((format!("{new}/{rest}"), *value));
            false
        } else {
            true
        }
    });
    for (key, value) in moved {
        data.expanded.insert(key, value);
    }
}

/// Drop folder expansion/selection state after a folder deletion.
fn drop_folder_state(data: &mut AppData, path: &str) {
    let prefix = format!("{path}/");
    data.expanded
        .retain(|key, _| key != path && !key.starts_with(&prefix));
    if data.selected_folder == path || data.selected_folder.starts_with(&prefix) {
        data.selected_folder = String::new();
    }
}

/// Rewrite open tab ids and the active note after a folder rename.
fn remap_tabs(ui: &AppWindow, old: &str, new: &str) {
    let prefix = format!("{old}/");
    let remap = |id: &str| -> String {
        if let Some(rest) = id.strip_prefix(&prefix) {
            format!("{new}/{rest}")
        } else if id == old {
            new.to_owned()
        } else {
            id.to_owned()
        }
    };

    let tabs_model = ui.get_open_tabs();
    let tabs: Vec<TabItem> = (0..tabs_model.row_count())
        .filter_map(|i| tabs_model.row_data(i))
        .map(|t| TabItem {
            id: remap(&t.id).into(),
            title: t.title,
            unsaved: t.unsaved,
        })
        .collect();
    ui.set_open_tabs(ModelRc::new(VecModel::from(tabs)));

    let current = ui.get_current_note_id().to_string();
    let remapped = remap(&current);
    if remapped != current {
        ui.set_current_note_id(remapped.into());
    }
}

/// Remove tabs (and clear the active note) that lived inside a deleted folder.
fn drop_tabs_in_folder(ui: &AppWindow, path: &str) {
    let prefix = format!("{path}/");
    let current = ui.get_current_note_id().to_string();
    if current.starts_with(&prefix) {
        ui.set_current_note_id("".into());
        ui.set_current_note_title("".into());
        ui.set_current_note_content("".into());
        render_preview(ui, "");
        ui.set_backlinks(ModelRc::new(VecModel::from(vec![])));
        ui.set_unsaved(false);
    }

    let tabs_model = ui.get_open_tabs();
    let tabs: Vec<TabItem> = (0..tabs_model.row_count())
        .filter_map(|i| tabs_model.row_data(i))
        .filter(|t| !t.id.starts_with(&prefix))
        .collect();
    ui.set_open_tabs(ModelRc::new(VecModel::from(tabs)));
}

/// Add a tab for `id`/`title` if it doesn't exist yet, then make it active.
fn add_or_activate_tab(ui: &AppWindow, id: &str, title: &str) {
    let tabs_model = ui.get_open_tabs();
    let count = tabs_model.row_count();

    for i in 0..count {
        if let Some(tab) = tabs_model.row_data(i) {
            if tab.id.as_str() == id {
                return;
            }
        }
    }

    let mut tabs: Vec<TabItem> = (0..count).filter_map(|i| tabs_model.row_data(i)).collect();
    tabs.push(TabItem {
        id: id.into(),
        title: title.into(),
        unsaved: false,
    });
    ui.set_open_tabs(ModelRc::new(VecModel::from(tabs)));
}

/// Remove the tab for `id`.
fn remove_tab(ui: &AppWindow, id: &str) {
    let tabs_model = ui.get_open_tabs();
    let count = tabs_model.row_count();
    let tabs: Vec<TabItem> = (0..count)
        .filter_map(|i| tabs_model.row_data(i))
        .filter(|t| t.id.as_str() != id)
        .collect();
    ui.set_open_tabs(ModelRc::new(VecModel::from(tabs)));
}

/// Set the `unsaved` flag on the tab with `id`.
fn mark_tab_unsaved(ui: &AppWindow, id: &str, unsaved: bool) {
    let tabs_model = ui.get_open_tabs();
    let count = tabs_model.row_count();
    let tabs: Vec<TabItem> = (0..count)
        .filter_map(|i| tabs_model.row_data(i))
        .map(|t| {
            if t.id.as_str() == id {
                TabItem {
                    id: t.id,
                    title: t.title,
                    unsaved,
                }
            } else {
                t
            }
        })
        .collect();
    ui.set_open_tabs(ModelRc::new(VecModel::from(tabs)));
}

/// Update a tab's id after its note was moved (the title is unchanged).
fn move_tab(ui: &AppWindow, old_id: &str, new_id: &str) {
    let tabs_model = ui.get_open_tabs();
    let count = tabs_model.row_count();
    let tabs: Vec<TabItem> = (0..count)
        .filter_map(|i| tabs_model.row_data(i))
        .map(|t| {
            if t.id.as_str() == old_id {
                TabItem {
                    id: new_id.into(),
                    title: t.title,
                    unsaved: t.unsaved,
                }
            } else {
                t
            }
        })
        .collect();
    ui.set_open_tabs(ModelRc::new(VecModel::from(tabs)));
}

/// Update the tab's id and title after a rename.
fn rename_tab(ui: &AppWindow, old_id: &str, new_id: &str, new_title: &str) {
    let tabs_model = ui.get_open_tabs();
    let count = tabs_model.row_count();
    let tabs: Vec<TabItem> = (0..count)
        .filter_map(|i| tabs_model.row_data(i))
        .map(|t| {
            if t.id.as_str() == old_id {
                TabItem {
                    id: new_id.into(),
                    title: new_title.into(),
                    unsaved: t.unsaved,
                }
            } else {
                t
            }
        })
        .collect();
    ui.set_open_tabs(ModelRc::new(VecModel::from(tabs)));
}
