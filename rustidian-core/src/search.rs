use crate::vault::{NoteId, NoteMeta};
use std::path::Path;
use walkdir::WalkDir;

/// A single search hit.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub meta: NoteMeta,
    /// First line of the note that contains the query (if any).
    pub excerpt: Option<String>,
}

/// Search for *query* (case-insensitive substring) in note titles **and**
/// contents.
///
/// Files are scanned one at a time to avoid loading the entire vault into RAM.
/// For a personal vault of hundreds of notes this is fast enough without an
/// index.
pub fn find(query: &str, vault: &Path) -> Vec<SearchResult> {
    if query.trim().is_empty() {
        return vec![];
    }

    let q = query.to_lowercase();
    let mut results = Vec::new();

    for entry in WalkDir::new(vault)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        let id: NoteId = path
            .strip_prefix(vault)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();

        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Check title match first (cheap).
        let title_match = title.to_lowercase().contains(&q);

        // Then scan content for match + excerpt.
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let excerpt: Option<String> = content
            .lines()
            .find(|line| line.to_lowercase().contains(&q))
            .map(|line| {
                // Trim to at most 120 chars so the UI doesn't overflow.
                let trimmed = line.trim();
                if trimmed.len() > 120 {
                    format!("{}…", &trimmed[..120])
                } else {
                    trimmed.to_owned()
                }
            });

        if title_match || excerpt.is_some() {
            results.push(SearchResult {
                meta: NoteMeta { id, title },
                excerpt,
            });
        }
    }

    results.sort_by(|a, b| {
        a.meta
            .title
            .to_lowercase()
            .cmp(&b.meta.title.to_lowercase())
    });
    results
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn finds_by_title() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        fs::write(vault.join("Rust Notes.md"), "some content").unwrap();
        fs::write(vault.join("Python.md"), "other content").unwrap();

        let hits = find("rust", vault);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].meta.title, "Rust Notes");
    }

    #[test]
    fn finds_by_content() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        fs::write(vault.join("A.md"), "petgraph is a graph library").unwrap();
        fs::write(vault.join("B.md"), "nothing relevant").unwrap();

        let hits = find("petgraph", vault);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].excerpt.is_some());
    }

    #[test]
    fn empty_query_returns_empty() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        fs::write(vault.join("A.md"), "hello").unwrap();

        assert!(find("", vault).is_empty());
        assert!(find("   ", vault).is_empty());
    }

    #[test]
    fn case_insensitive() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        fs::write(vault.join("Note.md"), "HELLO WORLD").unwrap();

        assert_eq!(find("hello", vault).len(), 1);
        assert_eq!(find("HELLO", vault).len(), 1);
    }
}
