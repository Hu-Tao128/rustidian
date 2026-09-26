mod bridge;
mod worker;

use bridge::{to_backlink_items, to_note_items};
use rustidian_core::config::Config;
use rustidian_core::links::normalise;
use rustidian_core::markdown::to_plain;
use rustidian_core::vault;
use slint::{Model, ModelRc, Timer, TimerMode, VecModel};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use worker::AppData;

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── Shared state (vault path starts empty) ────────────────────────────────
    let shared: Arc<Mutex<AppData>> = Arc::new(Mutex::new(AppData::default()));

    // ── Build UI ──────────────────────────────────────────────────────────────
    let ui = AppWindow::new()?;
    ui.set_graph_available(cfg!(feature = "graph"));
    // Default to split view (editor + preview side by side, like Obsidian)
    ui.set_view_mode(ViewMode::Split);

    // ── Decide whether to show the vault picker ───────────────────────────────
    //
    // Priority:
    //  1. RUSTIDIAN_VAULT env var (skips the picker entirely)
    //  2. Saved config file (~/.config/rustidian/config.toml)
    //  3. Show the picker so the user can choose
    let initial_vault: Option<PathBuf> = {
        // 1. Env var
        if let Ok(ev) = std::env::var("RUSTIDIAN_VAULT") {
            let p = PathBuf::from(ev);
            if p.is_dir() {
                Some(p)
            } else {
                None
            }
        } else {
            // 2. Saved config
            Config::load()
                .ok()
                .filter(|c| c.vault_path.is_dir())
                .map(|c| c.vault_path)
        }
    };

    if let Some(vault) = initial_vault {
        // We have a valid vault — open it right away, skip the picker.
        open_vault(vault, Arc::clone(&shared), ui.as_weak());
    } else {
        // No vault known — show the picker screen.
        ui.set_show_vault_picker(true);
    }

    // ── Helper: rebuild note list and refresh sidebar ─────────────────────────
    let ui_weak = ui.as_weak();
    let shared_clone = Arc::clone(&shared);
    let refresh_note_list = move || {
        let ui = match ui_weak.upgrade() {
            Some(u) => u,
            None => return,
        };
        let data = shared_clone.lock().unwrap();
        let items: Vec<NoteItem> = to_note_items(&data.notes);
        ui.set_notes(ModelRc::new(VecModel::from(items)));

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
    };

    // ── vault-open-requested ──────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        let refresh = refresh_note_list.clone();
        ui.on_vault_open_requested(move |path_str| {
            let path = PathBuf::from(path_str.as_str().trim());

            // Create the directory if it doesn't exist yet.
            if !path.exists() {
                if let Err(e) = std::fs::create_dir_all(&path) {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_vault_picker_error(
                            format!("Cannot create folder: {e}").into(),
                        );
                    }
                    return;
                }
            }

            if !path.is_dir() {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_vault_picker_error(
                        "That path is not a folder. Please enter a folder path.".into(),
                    );
                }
                return;
            }

            // Persist the choice.
            let mut config = Config::load().unwrap_or(Config {
                vault_path: path.clone(),
            });
            config.vault_path = path.clone();
            let _ = config.save();

            // Clear any picker error and close the picker screen.
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_vault_picker_error("".into());
                ui.set_show_vault_picker(false);
                ui.set_status_kind(StatusKind::Info);
                ui.set_status_message(format!("Vault: {}", path.display()).into());
                // Clear tabs when switching vault
                ui.set_open_tabs(ModelRc::new(VecModel::from(vec![])));
            }

            // Load the vault.
            open_vault(path.clone(), Arc::clone(&shared2), ui_weak.clone());

            // Trigger sidebar refresh after index is built.
            let refresh2 = refresh.clone();
            let shared3 = Arc::clone(&shared2);
            worker::rebuild_index(path, shared3, refresh2);
        });
    }

    // ── Initial vault scan (if we already had a vault) ────────────────────────
    {
        let vault_now = shared.lock().unwrap().vault_path.clone();
        if !vault_now.as_os_str().is_empty() {
            let shared2 = Arc::clone(&shared);
            let refresh = refresh_note_list.clone();
            worker::rebuild_index(vault_now, shared2, refresh);
        }
    }

    // ── note-selected ─────────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        ui.on_note_selected(move |id| {
            let id_str = id.to_string();
            let data = shared2.lock().unwrap();

            // Auto-save the current note before switching so no content is lost.
            {
                if let Some(ui) = ui_weak.upgrade() {
                    let cur_id = ui.get_current_note_id().to_string();
                    let cur_content = ui.get_current_note_content().to_string();
                    if !cur_id.is_empty() && ui.get_unsaved() {
                        let _ = vault::write_note(&data.vault_path, &cur_id, &cur_content);
                    }
                }
            }

            match vault::read_note(&data.vault_path, &id_str) {
                Ok(note) => {
                    let ui = match ui_weak.upgrade() {
                        Some(u) => u,
                        None => return,
                    };
                    ui.set_current_note_id(note.meta.id.clone().into());
                    ui.set_current_note_title(note.meta.title.clone().into());
                    ui.set_current_note_content(note.content.clone().into());
                    // Render Markdown to clean plain text for the preview pane
                    ui.set_preview_html(to_plain(&note.content).into());
                    ui.set_unsaved(false);
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message(format!("Opened: {}", note.meta.title).into());

                    let bl_ids = data
                        .link_index
                        .backlinks
                        .get(&normalise(&note.meta.id))
                        .cloned()
                        .unwrap_or_default();
                    let bl_items = to_backlink_items(&bl_ids, &data.notes);
                    ui.set_backlinks(ModelRc::new(VecModel::from(bl_items)));

                    // Add or activate tab
                    add_or_activate_tab(&ui, &note.meta.id, &note.meta.title);
                }
                Err(e) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_status_kind(StatusKind::Error);
                        ui.set_status_message(format!("Error opening note: {e}").into());
                    }
                }
            }
        });
    }

    // ── content-changed (marks unsaved + updates preview + debounce autosave) ──
    {
        let ui_weak = ui.as_weak();
        let timer_rc = Rc::new(Timer::default());
        let shared2 = Arc::clone(&shared);
        ui.on_content_changed(move |text| {
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            // Render Markdown to plain text for live preview
            ui.set_preview_html(to_plain(text.as_str()).into());
            ui.set_unsaved(true);

            // Mark the active tab as unsaved
            let cur_id = ui.get_current_note_id().to_string();
            mark_tab_unsaved(&ui, &cur_id, true);

            let ui_weak2 = ui_weak.clone();
            let shared3 = Arc::clone(&shared2);
            timer_rc.start(
                TimerMode::SingleShot,
                std::time::Duration::from_millis(600),
                {
                    let ui_weak3 = ui_weak2.clone();
                    move || {
                        let ui = match ui_weak3.upgrade() {
                            Some(u) => u,
                            None => return,
                        };
                        let id = ui.get_current_note_id().to_string();
                        let content = ui.get_current_note_content().to_string();
                        if id.is_empty() {
                            return;
                        }
                        let data = shared3.lock().unwrap();
                        match vault::write_note(&data.vault_path, &id, &content) {
                            Ok(()) => {
                                ui.set_unsaved(false);
                                mark_tab_unsaved(&ui, &id, false);
                                ui.set_status_kind(StatusKind::Success);
                                ui.set_status_message("Saved.".into());
                            }
                            Err(e) => {
                                ui.set_status_kind(StatusKind::Error);
                                ui.set_status_message(format!("Save error: {e}").into());
                            }
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
        let refresh = refresh_note_list.clone();
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
                    let shared3 = Arc::clone(&shared2);
                    let r2 = refresh.clone();
                    worker::rebuild_index(vault, shared3, r2);
                }
                Err(e) => {
                    ui.set_status_kind(StatusKind::Error);
                    ui.set_status_message(format!("Save error: {e}").into());
                }
            }
        });
    }

    // ── new-note-requested ────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        let refresh = refresh_note_list.clone();
        ui.on_new_note_requested(move || {
            let vault = {
                let data = shared2.lock().unwrap();
                data.vault_path.clone()
            };
            if vault.as_os_str().is_empty() {
                return; // No vault open yet
            }
            let title = next_untitled_name(&vault);
            match vault::create_note(&vault, &title) {
                Ok(new_id) => {
                    let ui = match ui_weak.upgrade() {
                        Some(u) => u,
                        None => return,
                    };
                    ui.set_current_note_id(new_id.clone().into());
                    ui.set_current_note_title(title.clone().into());
                    ui.set_current_note_content("".into());
                    ui.set_preview_html("".into());
                    ui.set_unsaved(false);
                    ui.set_backlinks(ModelRc::new(VecModel::from(vec![])));
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message(format!("Created: {title}").into());
                    // Add a tab for the new note
                    add_or_activate_tab(&ui, &new_id, &title);
                    let shared3 = Arc::clone(&shared2);
                    let r2 = refresh.clone();
                    worker::rebuild_index(vault, shared3, r2);
                }
                Err(e) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_status_kind(StatusKind::Error);
                        ui.set_status_message(format!("Error creating note: {e}").into());
                    }
                }
            }
        });
    }

    // ── delete-requested ─────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let shared2 = Arc::clone(&shared);
        let refresh = refresh_note_list.clone();
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
                    // Remove the tab
                    remove_tab(&ui, &id_str);
                    if ui.get_current_note_id().as_str() == id_str {
                        ui.set_current_note_id("".into());
                        ui.set_current_note_title("".into());
                        ui.set_current_note_content("".into());
                        ui.set_preview_html("".into());
                        ui.set_backlinks(ModelRc::new(VecModel::from(vec![])));
                    }
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message("Note deleted.".into());
                    let shared3 = Arc::clone(&shared2);
                    let r2 = refresh.clone();
                    worker::rebuild_index(vault, shared3, r2);
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
        let refresh = refresh_note_list.clone();
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
                    // Update the tab title too
                    rename_tab(&ui, &id_str, &new_id, &new_title_str);
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message(format!("Renamed to \"{new_title_str}\"").into());
                    let shared3 = Arc::clone(&shared2);
                    let r2 = refresh.clone();
                    worker::rebuild_index(vault, shared3, r2);
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

            // Save before closing if unsaved
            {
                let data = shared2.lock().unwrap();
                if ui.get_current_note_id().as_str() == id_str && ui.get_unsaved() {
                    let content = ui.get_current_note_content().to_string();
                    let _ = vault::write_note(&data.vault_path, &id_str, &content);
                }
            }

            let was_active = ui.get_current_note_id().as_str() == id_str;
            remove_tab(&ui, &id_str);

            // If we closed the active tab, switch to the last remaining tab
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
                        ui.set_preview_html(to_plain(&note.content).into());
                        ui.set_unsaved(false);
                    }
                } else {
                    // No tabs left — clear the editor
                    ui.set_current_note_id("".into());
                    ui.set_current_note_title("".into());
                    ui.set_current_note_content("".into());
                    ui.set_preview_html("".into());
                    ui.set_unsaved(false);
                    ui.set_backlinks(ModelRc::new(VecModel::from(vec![])));
                }
            }
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
                                .map(|r| {
                                    let folder = {
                                        let p = std::path::Path::new(&r.meta.id);
                                        match p.parent() {
                                            Some(par) if par.as_os_str() != "" => {
                                                par.to_string_lossy().into_owned()
                                            }
                                            _ => String::new(),
                                        }
                                    };
                                    NoteItem {
                                        id: r.meta.id.into(),
                                        title: r.meta.title.into(),
                                        folder: folder.into(),
                                    }
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

    ui.run()?;
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Set the vault path in shared state and update the UI status bar.
/// Does NOT rebuild the index — callers do that separately.
fn open_vault(vault: PathBuf, shared: Arc<Mutex<AppData>>, ui_weak: slint::Weak<AppWindow>) {
    {
        let mut data = shared.lock().unwrap();
        data.vault_path = vault.clone();
    }
    if let Some(ui) = ui_weak.upgrade() {
        ui.set_status_kind(StatusKind::Info);
        ui.set_status_message(format!("Vault: {}", vault.display()).into());
    }
}

/// Return an "Untitled N" name that does not collide with existing notes.
fn next_untitled_name(vault: &std::path::Path) -> String {
    let mut n = 1u32;
    loop {
        let name = if n == 1 {
            "Untitled".to_owned()
        } else {
            format!("Untitled {n}")
        };
        if !vault.join(format!("{name}.md")).exists() {
            return name;
        }
        n += 1;
    }
}

/// Add a tab for `id`/`title` if it doesn't exist yet, then make it active.
fn add_or_activate_tab(ui: &AppWindow, id: &str, title: &str) {
    let tabs_model = ui.get_open_tabs();
    let count = tabs_model.row_count();

    // Check if already open
    for i in 0..count {
        if let Some(tab) = tabs_model.row_data(i) {
            if tab.id.as_str() == id {
                return; // already in the bar
            }
        }
    }

    // Append new tab
    let mut tabs: Vec<TabItem> = (0..count)
        .filter_map(|i| tabs_model.row_data(i))
        .collect();
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
