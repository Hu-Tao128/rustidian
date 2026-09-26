use rustidian_core::vault::{NoteId, NoteMeta};

use crate::{BacklinkItem, NoteItem};

/// Convert a [`NoteMeta`] list to the Slint-compatible [`NoteItem`] model.
pub fn to_note_items(notes: &[NoteMeta]) -> Vec<NoteItem> {
    notes
        .iter()
        .map(|n| {
            // The id is a relative path like "subfolder/note.md" or just "note.md".
            // Extract the parent folder (empty string for root-level notes).
            let folder = {
                let p = std::path::Path::new(&n.id);
                match p.parent() {
                    Some(parent) if parent.as_os_str() != "" => {
                        parent.to_string_lossy().into_owned()
                    }
                    _ => String::new(),
                }
            };
            NoteItem {
                id: n.id.clone().into(),
                title: n.title.clone().into(),
                folder: folder.into(),
            }
        })
        .collect()
}

/// Convert a backlinks list (vec of NoteId + title) to [`BacklinkItem`] model.
pub fn to_backlink_items(ids: &[NoteId], all_notes: &[NoteMeta]) -> Vec<BacklinkItem> {
    ids.iter()
        .map(|id| {
            let title = all_notes
                .iter()
                .find(|n| n.id.to_lowercase() == id.to_lowercase())
                .map(|n| n.title.clone())
                .unwrap_or_else(|| id.clone());
            BacklinkItem {
                id: id.clone().into(),
                title: title.into(),
            }
        })
        .collect()
}
