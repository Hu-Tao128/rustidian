use rustidian_core::links::LinkIndex;
use rustidian_core::vault::NoteMeta;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Shared application state accessed from both the main thread (UI) and
/// background workers.
#[derive(Debug, Default)]
pub struct AppData {
    pub notes: Vec<NoteMeta>,
    pub link_index: LinkIndex,
    pub vault_path: PathBuf,
}

/// Spawn a background thread that scans the vault and rebuilds the note list
/// and link index, then invokes *on_done* on the Slint event loop.
pub fn rebuild_index(
    vault: PathBuf,
    shared: Arc<Mutex<AppData>>,
    on_done: impl Fn() + Send + 'static,
) {
    std::thread::spawn(move || {
        let notes = match rustidian_core::vault::list_notes(&vault) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("[worker] list_notes error: {e}");
                return;
            }
        };
        let index = rustidian_core::links::build_index(&vault);

        {
            let mut data = shared.lock().unwrap();
            data.notes = notes;
            data.link_index = index;
            data.vault_path = vault;
        }

        // Schedule the UI refresh on the event loop — Slint is not Send.
        slint::invoke_from_event_loop(on_done).ok();
    });
}

/// Spawn a background search and invoke *on_done* with the results.
pub fn run_search(
    query: String,
    vault: PathBuf,
    on_done: impl Fn(Vec<rustidian_core::search::SearchResult>) + Send + 'static,
) {
    std::thread::spawn(move || {
        let results = rustidian_core::search::find(&query, &vault);
        slint::invoke_from_event_loop(move || on_done(results)).ok();
    });
}
