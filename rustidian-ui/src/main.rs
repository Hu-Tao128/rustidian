mod bridge;
mod editor_assist;
mod worker;

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
    // ── Shared state (vault path starts empty) ────────────────────────────────
    let shared: Arc<Mutex<AppData>> = Arc::new(Mutex::new(AppData::default()));

    // ── Build UI ──────────────────────────────────────────────────────────────
    let ui = AppWindow::new()?;
    ui.set_graph_available(cfg!(feature = "graph"));
    ui.set_view_mode(ViewMode::Split);

    // ── Theme ─────────────────────────────────────────────────────────────────
    let config = Config::load().ok();
    let dark = config.as_ref().map(|c| c.dark_mode).unwrap_or(true);
    ui.set_dark_mode(dark);
    apply_theme(&ui, dark);

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

            // Auto-save the current note before switching so no content is lost.
            if let Some(ui) = ui_weak.upgrade() {
                let cur_id = ui.get_current_note_id().to_string();
                let cur_content = ui.get_current_note_content().to_string();
                if !cur_id.is_empty() && ui.get_unsaved() {
                    let vault_path = shared2.lock().unwrap().vault_path.clone();
                    let _ = vault::write_note(&vault_path, &cur_id, &cur_content);
                }
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
                let entry = data.expanded.entry(path_str.clone()).or_insert(true);
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

    // ── content-changed (marks unsaved + updates preview + debounce autosave) ─
    {
        let ui_weak = ui.as_weak();
        let timer_rc = Rc::new(Timer::default());
        let shared2 = Arc::clone(&shared);
        ui.on_content_changed(move |text| {
            let ui = match ui_weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            render_preview(&ui, text.as_str());
            ui.set_unsaved(true);

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
                    worker::rebuild_index(vault, Arc::clone(&shared2), refresh);
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
        ui.on_new_note_requested(move || {
            let (vault, folder) = {
                let data = shared2.lock().unwrap();
                (data.vault_path.clone(), data.selected_folder.clone())
            };
            if vault.as_os_str().is_empty() {
                return; // No vault open yet
            }
            let title = next_untitled_name(&vault, &folder);
            match vault::create_note_in(&vault, &folder, &title) {
                Ok(new_id) => {
                    let ui = match ui_weak.upgrade() {
                        Some(u) => u,
                        None => return,
                    };
                    ui.set_current_note_id(new_id.clone().into());
                    ui.set_current_note_title(title.clone().into());
                    ui.set_current_note_content("".into());
                    render_preview(&ui, "");
                    ui.set_unsaved(false);
                    ui.set_backlinks(ModelRc::new(VecModel::from(vec![])));
                    ui.set_status_kind(StatusKind::Success);
                    ui.set_status_message(format!("Created: {title}").into());
                    add_or_activate_tab(&ui, &new_id, &title);
                    let refresh = make_refresh(ui_weak.clone(), Arc::clone(&shared2));
                    worker::rebuild_index(vault, Arc::clone(&shared2), refresh);
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
        p.set_success(rgb(0x40a02b));
        p.set_danger(rgb(0xd20f39));
        p.set_warning(rgb(0xdf8e1d));
    }
}

/// Render Markdown content into the preview block model.
fn render_preview(ui: &AppWindow, content: &str) {
    let items = blocks_to_items(&parse_blocks(content));
    ui.set_preview_blocks(ModelRc::new(VecModel::from(items)));
}

/// Rebuild the sidebar tree, note titles and backlinks from shared state.
fn refresh_sidebar(ui: &AppWindow, data: &AppData) {
    let rows = tree_to_rows(&data.tree, &data.expanded, &data.selected_folder);
    ui.set_note_count(data.notes.len() as i32);
    ui.set_tree(ModelRc::new(VecModel::from(rows)));

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

/// Set the vault path in shared state and update the UI status bar.
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

/// Persist the chosen vault, switch the UI to the main layout and load it.
fn activate_vault(path: PathBuf, shared: Arc<Mutex<AppData>>, ui_weak: slint::Weak<AppWindow>) {
    let mut config = Config::load().unwrap_or(Config {
        vault_path: path.clone(),
        dark_mode: true,
    });
    config.vault_path = path.clone();
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
