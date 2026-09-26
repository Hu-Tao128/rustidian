use crate::vault::NoteId;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;
use walkdir::WalkDir;

// Compiled once — never inside a loop.
fn wikilink_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    // Matches [[Target]] and [[Target|Alias]]
    RE.get_or_init(|| regex::Regex::new(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]").unwrap())
}

/// Bidirectional link index for the entire vault.
///
/// `outgoing[A]` = notes that A links to.
/// `backlinks[B]` = notes that link to B.
#[derive(Debug, Clone, Default)]
pub struct LinkIndex {
    pub outgoing: HashMap<NoteId, Vec<NoteId>>,
    pub backlinks: HashMap<NoteId, Vec<NoteId>>,
}

/// Normalise a wikilink target so that `[[Note]]` and `[[note]]` resolve to
/// the same file.
pub fn normalise(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Build a full [`LinkIndex`] by scanning every `.md` file in *vault*.
///
/// The index is built in two passes:
/// 1. Collect all `outgoing` links from each note.
/// 2. Invert to produce `backlinks`.
///
/// Broken links (pointing at a non-existing note) are stored but not resolved —
/// they will simply have no matching note in `list_notes`.
pub fn build_index(vault: &Path) -> LinkIndex {
    let re = wikilink_re();
    let mut outgoing: HashMap<NoteId, Vec<NoteId>> = HashMap::new();

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

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let targets: Vec<NoteId> = re
            .captures_iter(&content)
            .map(|cap| {
                let target = cap[1].trim();
                // If the target has no extension, assume `.md`
                if target.contains('.') {
                    normalise(target)
                } else {
                    format!("{}.md", normalise(target))
                }
            })
            .collect();

        outgoing.insert(id, targets);
    }

    // Invert to build backlinks.
    let mut backlinks: HashMap<NoteId, Vec<NoteId>> = HashMap::new();
    for (source, targets) in &outgoing {
        for target in targets {
            backlinks
                .entry(target.clone())
                .or_default()
                .push(source.clone());
        }
    }

    LinkIndex {
        outgoing,
        backlinks,
    }
}

/// Return the list of note IDs that link **to** *id*.
pub fn backlinks_for<'a>(index: &'a LinkIndex, id: &str) -> &'a [NoteId] {
    index
        .backlinks
        .get(&normalise(id))
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn simple_link() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        fs::write(vault.join("A.md"), "see [[B]]").unwrap();
        fs::write(vault.join("B.md"), "hello").unwrap();

        let idx = build_index(vault);
        let out = &idx.outgoing["A.md"];
        assert!(out.contains(&"b.md".to_string()));

        // B has a backlink from A
        assert!(idx.backlinks["b.md"].contains(&"A.md".to_string()));
    }

    #[test]
    fn link_with_alias() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        fs::write(vault.join("A.md"), "[[B|click here]]").unwrap();
        fs::write(vault.join("B.md"), "").unwrap();

        let idx = build_index(vault);
        assert!(idx.outgoing["A.md"].contains(&"b.md".to_string()));
    }

    #[test]
    fn broken_link_does_not_crash() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        fs::write(vault.join("A.md"), "[[Ghost]]").unwrap();

        let idx = build_index(vault);
        // A has the outgoing link even though Ghost doesn't exist
        assert!(idx.outgoing["A.md"].contains(&"ghost.md".to_string()));
    }

    #[test]
    fn mutual_links_no_infinite_loop() {
        let dir = tempdir().unwrap();
        let vault = dir.path();
        fs::write(vault.join("A.md"), "[[B]]").unwrap();
        fs::write(vault.join("B.md"), "[[A]]").unwrap();

        // Must not hang or stack-overflow.
        let idx = build_index(vault);
        assert!(idx.outgoing.contains_key("A.md"));
        assert!(idx.outgoing.contains_key("B.md"));
    }
}
