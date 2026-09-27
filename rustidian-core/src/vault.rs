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

/// A node in the vault's folder tree.
///
/// `name` is the display name of a single folder segment (empty for the vault
/// root), `path` is the full path relative to the vault root (empty for the
/// root), `notes` are the notes stored directly in this folder and `children`
/// are the sub-folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderNode {
    pub name: String,
    pub path: String,
    pub notes: Vec<NoteMeta>,
    pub children: Vec<FolderNode>,
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

/// Scan *vault* and return its folder hierarchy as a tree.
///
/// The result is always a single-element vector containing the vault root
/// (`name == ""`, `path == ""`).  The root's `notes` are the notes stored
/// directly in the vault folder and its `children` are the sub-folders,
/// sorted alphabetically.  Notes inside each folder are sorted by title.
pub fn list_notes_tree(vault: &Path) -> Result<Vec<FolderNode>, CoreError> {
    let notes = list_notes(vault)?;
    let mut root = FolderNode {
        name: String::new(),
        path: String::new(),
        notes: Vec::new(),
        children: Vec::new(),
    };

    for note in notes {
        let parent = Path::new(&note.id)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if parent.is_empty() {
            root.notes.push(note);
        } else {
            let segments: Vec<&str> = parent.split('/').collect();
            insert_note(&mut root, &segments, note);
        }
    }

    sort_tree(&mut root);
    Ok(vec![root])
}

/// Recursively insert *note* into the child folder described by *segments*.
fn insert_note(node: &mut FolderNode, segments: &[&str], note: NoteMeta) {
    if segments.is_empty() {
        node.notes.push(note);
        return;
    }
    let segment = segments[0];
    let child_path = if node.path.is_empty() {
        segment.to_owned()
    } else {
        format!("{}/{}", node.path, segment)
    };
    let index = match node.children.iter().position(|c| c.name == segment) {
        Some(i) => i,
        None => {
            node.children.push(FolderNode {
                name: segment.to_owned(),
                path: child_path,
                notes: Vec::new(),
                children: Vec::new(),
            });
            node.children.len() - 1
        }
    };
    insert_note(&mut node.children[index], &segments[1..], note);
}

/// Sort a folder tree: children by name, notes by title (both case-insensitive).
fn sort_tree(node: &mut FolderNode) {
    node.notes.sort_by_key(|n| n.title.to_lowercase());
    node.children.sort_by_key(|c| c.name.to_lowercase());
    for child in &mut node.children {
        sort_tree(child);
    }
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

/// Create a brand-new note at the vault root with the given *title* (no `.md`
/// extension needed).
///
/// Returns the [`NoteId`] of the newly created note.
pub fn create_note(vault: &Path, title: &str) -> Result<NoteId, CoreError> {
    create_note_in(vault, "", title)
}

/// Create a brand-new note inside *folder* (relative to the vault root, `""`
/// for the root itself).  Missing intermediate folders are created.
///
/// Returns the [`NoteId`] of the newly created note.
pub fn create_note_in(vault: &Path, folder: &str, title: &str) -> Result<NoteId, CoreError> {
    let dir = if folder.trim().is_empty() {
        vault.to_path_buf()
    } else {
        vault.join(folder.trim())
    };
    let filename = format!("{}.md", title.trim());
    let path = dir.join(&filename);
    if path.exists() {
        return Err(CoreError::NameCollision(relative_id(vault, &path)));
    }
    // Ensure the target directory (and any parents) exists.
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&path, "")?;
    Ok(relative_id(vault, &path))
}

/// Rename an existing note, keeping it in its current folder.  Fails if the
/// new name already exists in that folder.
///
/// Returns the new [`NoteId`].
pub fn rename_note(vault: &Path, id: &str, new_title: &str) -> Result<NoteId, CoreError> {
    let old_path = absolute_path(vault, id);
    if !old_path.exists() {
        return Err(CoreError::NotFound(id.to_owned()));
    }
    let parent = old_path.parent().unwrap_or(vault);
    let new_path = parent.join(format!("{}.md", new_title.trim()));
    if new_path.exists() {
        return Err(CoreError::NameCollision(relative_id(vault, &new_path)));
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

    #[test]
    fn list_notes_scans_subfolders() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        create_note_in(vault, "Proyectos/2024", "Roadmap").unwrap();

        let notes = list_notes(vault).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, "Proyectos/2024/Roadmap.md");
    }

    #[test]
    fn create_note_in_creates_missing_folders() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        let id = create_note_in(vault, "A/B", "Deep").unwrap();
        assert_eq!(id, "A/B/Deep.md");
        assert!(vault.join("A/B/Deep.md").exists());
    }

    #[test]
    fn list_notes_tree_groups_by_folder() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        create_note(vault, "Root note").unwrap();
        create_note_in(vault, "Proyectos", "Alpha").unwrap();
        create_note_in(vault, "Proyectos/2024", "Beta").unwrap();

        let tree = list_notes_tree(vault).unwrap();
        assert_eq!(tree.len(), 1);
        let root = &tree[0];
        assert_eq!(root.path, "");
        assert_eq!(root.notes.len(), 1);
        assert_eq!(root.notes[0].title, "Root note");
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].name, "Proyectos");
        assert_eq!(root.children[0].notes.len(), 1);
        assert_eq!(root.children[0].children.len(), 1);
        assert_eq!(root.children[0].children[0].path, "Proyectos/2024");
        assert_eq!(root.children[0].children[0].notes[0].title, "Beta");
    }

    #[test]
    fn rename_keeps_note_in_folder() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        let id = create_note_in(vault, "Folder", "Old").unwrap();
        write_note(vault, &id, "content").unwrap();

        let new_id = rename_note(vault, &id, "New").unwrap();
        assert_eq!(new_id, "Folder/New.md");
        assert_eq!(read_note(vault, &new_id).unwrap().content, "content");
        assert!(read_note(vault, &id).is_err());
    }
}
