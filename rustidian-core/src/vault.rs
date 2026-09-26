use crate::CoreError;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Stable identifier for a note: its path relative to the vault root.
///
/// **Known limitation (v1):** renaming a note breaks any `[[links]]` that
/// pointed to it — the same behaviour as Obsidian without plugins.  This is a
/// deliberate trade-off; adding stable IDs via frontmatter would be
/// over-engineering for v1.
pub type NoteId = String;

/// Light-weight metadata about a note — loaded on startup for every note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteMeta {
    /// Relative path from the vault root (the note's stable identifier).
    pub id: NoteId,
    /// File name without extension, used as display title.
    pub title: String,
}

/// Full note contents plus metadata.
#[derive(Debug, Clone)]
pub struct Note {
    pub meta: NoteMeta,
    pub content: String,
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Derive a [`NoteId`] (relative path string) from an absolute path and the
/// vault root.
fn relative_id(vault: &Path, absolute: &Path) -> NoteId {
    absolute
        .strip_prefix(vault)
        .unwrap_or(absolute)
        .to_string_lossy()
        .into_owned()
}

/// Derive a display title from the file stem (name without extension).
fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Build the absolute path for a given [`NoteId`] inside a vault.
fn absolute_path(vault: &Path, id: &str) -> PathBuf {
    vault.join(id)
}

// ── public API ───────────────────────────────────────────────────────────────

/// Scan *vault* and return lightweight metadata for every `.md` file found,
/// sorted alphabetically by title.
///
/// Only file names and paths are read — note contents are **not** loaded into
/// memory.
pub fn list_notes(vault: &Path) -> Result<Vec<NoteMeta>, CoreError> {
    let mut notes = Vec::new();

    for entry in WalkDir::new(vault)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            notes.push(NoteMeta {
                id: relative_id(vault, path),
                title: title_from_path(path),
            });
        }
    }

    notes.sort_by_key(|a| a.title.to_lowercase());
    Ok(notes)
}

/// Read the full content of the note identified by *id*.
pub fn read_note(vault: &Path, id: &str) -> Result<Note, CoreError> {
    let path = absolute_path(vault, id);
    if !path.exists() {
        return Err(CoreError::NotFound(id.to_owned()));
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(Note {
        meta: NoteMeta {
            id: id.to_owned(),
            title: title_from_path(&path),
        },
        content,
    })
}

/// Overwrite the content of an existing note.
pub fn write_note(vault: &Path, id: &str, content: &str) -> Result<(), CoreError> {
    let path = absolute_path(vault, id);
    if !path.exists() {
        return Err(CoreError::NotFound(id.to_owned()));
    }
    std::fs::write(path, content)?;
    Ok(())
}

/// Create a brand-new note with the given *title* (no `.md` extension needed).
///
/// Returns the [`NoteId`] of the newly created note.
pub fn create_note(vault: &Path, title: &str) -> Result<NoteId, CoreError> {
    let filename = format!("{}.md", title.trim());
    let path = vault.join(&filename);
    if path.exists() {
        return Err(CoreError::NameCollision(filename));
    }
    // Ensure the vault directory exists.
    std::fs::create_dir_all(vault)?;
    std::fs::write(&path, "")?;
    Ok(relative_id(vault, &path))
}

/// Rename an existing note.  Fails if the new name already exists.
///
/// Returns the new [`NoteId`].
pub fn rename_note(vault: &Path, id: &str, new_title: &str) -> Result<NoteId, CoreError> {
    let old_path = absolute_path(vault, id);
    if !old_path.exists() {
        return Err(CoreError::NotFound(id.to_owned()));
    }
    let new_filename = format!("{}.md", new_title.trim());
    let new_path = vault.join(&new_filename);
    if new_path.exists() {
        return Err(CoreError::NameCollision(new_filename));
    }
    std::fs::rename(&old_path, &new_path)?;
    Ok(relative_id(vault, &new_path))
}

/// Permanently delete a note.
pub fn delete_note(vault: &Path, id: &str) -> Result<(), CoreError> {
    let path = absolute_path(vault, id);
    if !path.exists() {
        return Err(CoreError::NotFound(id.to_owned()));
    }
    std::fs::remove_file(path)?;
    Ok(())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn create_and_list() {
        let dir = tempdir().unwrap();
        let vault = dir.path();

        create_note(vault, "Alpha").unwrap();
        create_note(vault, "Beta").unwrap();

        let notes = list_notes(vault).unwrap();
        let titles: Vec<&str> = notes.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(titles, ["Alpha", "Beta"]);
    }

    #[test]
    fn read_and_write() {
        let dir = tempdir().unwrap();
        let vault = dir.path();

        let id = create_note(vault, "Hello").unwrap();
        write_note(vault, &id, "# Hello\nworld").unwrap();

        let note = read_note(vault, &id).unwrap();
        assert_eq!(note.content, "# Hello\nworld");
        assert_eq!(note.meta.title, "Hello");
    }

    #[test]
    fn delete_note_removes_file() {
        let dir = tempdir().unwrap();
        let vault = dir.path();

        let id = create_note(vault, "Temp").unwrap();
        delete_note(vault, &id).unwrap();

        assert!(list_notes(vault).unwrap().is_empty());
    }

    #[test]
    fn rename_note_succeeds() {
        let dir = tempdir().unwrap();
        let vault = dir.path();

        let id = create_note(vault, "Old").unwrap();
        write_note(vault, &id, "content").unwrap();
        let new_id = rename_note(vault, &id, "New").unwrap();

        assert_eq!(read_note(vault, &new_id).unwrap().content, "content");
        assert!(read_note(vault, &id).is_err());
    }

    #[test]
    fn rename_collision_returns_error() {
        let dir = tempdir().unwrap();
        let vault = dir.path();

        let id = create_note(vault, "A").unwrap();
        create_note(vault, "B").unwrap();

        let err = rename_note(vault, &id, "B").unwrap_err();
        assert!(matches!(err, CoreError::NameCollision(_)));
    }

    #[test]
    fn create_duplicate_returns_error() {
        let dir = tempdir().unwrap();
        let vault = dir.path();

        create_note(vault, "Dup").unwrap();
        let err = create_note(vault, "Dup").unwrap_err();
        assert!(matches!(err, CoreError::NameCollision(_)));
    }

    #[test]
    fn read_nonexistent_returns_error() {
        let dir = tempdir().unwrap();
        let err = read_note(dir.path(), "ghost.md").unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[test]
    fn list_notes_ignores_non_md_files() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        fs::write(vault.join("readme.txt"), "not a note").unwrap();
        create_note(vault, "Real").unwrap();

        let notes = list_notes(vault).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "Real");
    }
}
