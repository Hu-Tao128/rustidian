//! Markdown editing assistance: list continuation, auto-closing pairs and
//! wikilink completion.
//!
//! Everything here is pure string manipulation and lives in the UI crate on
//! purpose — `rustidian-core` knows nothing about editing behaviour.

use crate::EditResult;

/// Clamp a byte offset coming from Slint to a valid char boundary.
fn clamp(offset: i32, text: &str) -> usize {
    let mut o = (offset.max(0) as usize).min(text.len());
    while o > 0 && !text.is_char_boundary(o) {
        o -= 1;
    }
    o
}

fn handled(text: String, cursor: usize) -> EditResult {
    EditResult {
        handled: true,
        text: text.into(),
        cursor: cursor as i32,
        anchor: cursor as i32,
    }
}

fn not_handled(content: &str, cursor: i32, anchor: i32) -> EditResult {
    EditResult {
        handled: false,
        text: content.into(),
        cursor,
        anchor,
    }
}

/// Splice *insert* into *content* at the (clamped) byte offset *cursor* and put
/// the caret after the inserted text.
pub fn insert_at(content: &str, cursor: i32, insert: &str) -> EditResult {
    let pos = clamp(cursor, content);
    let mut out = String::with_capacity(content.len() + insert.len());
    out.push_str(&content[..pos]);
    out.push_str(insert);
    out.push_str(&content[pos..]);
    handled(out, pos + insert.len())
}

/// If the cursor sits inside an unterminated `[[…`, return the filter text
/// typed so far (the part after `[[`).
pub fn wikilink_query(text: &str, cursor: usize) -> Option<&str> {
    let cursor = cursor.min(text.len());
    let before = &text[..cursor];
    let open = before.rfind("[[")?;
    let after = &before[open + 2..];
    if after.contains("]]") || after.contains('\n') {
        return None;
    }
    Some(after)
}

fn wikilink_start(text: &str, cursor: usize) -> Option<usize> {
    let cursor = cursor.min(text.len());
    let before = &text[..cursor];
    let open = before.rfind("[[")?;
    let after = &before[open + 2..];
    if after.contains("]]") || after.contains('\n') {
        return None;
    }
    Some(open)
}

/// Compute the wikilink suggestions for the current cursor position.
pub fn suggestions(content: &str, cursor: usize, titles: &[String]) -> (Vec<String>, bool) {
    match wikilink_query(content, cursor) {
        Some(filter) => {
            let filter = filter.to_lowercase();
            let mut matches: Vec<String> = titles
                .iter()
                .filter(|t| filter.is_empty() || t.to_lowercase().contains(&filter))
                .cloned()
                .collect();
            matches.sort_by_key(|t| t.to_lowercase());
            matches.truncate(8);
            (matches, true)
        }
        None => (Vec::new(), false),
    }
}

/// Handle a key press in the editor.  Returns an [`EditResult`] describing
/// whether the event was consumed and, if so, the resulting text/cursor.
pub fn handle_key(
    key: &str,
    content: &str,
    cursor: i32,
    anchor: i32,
    titles: &[String],
) -> EditResult {
    let cur = clamp(cursor, content);
    let anc = clamp(anchor, content);

    // 1. Accept a wikilink suggestion with Tab/Enter.
    if (key == "\t" || key == "\n") && wikilink_query(content, cur).is_some() {
        if let Some((text, cursor)) = accept_wikilink(content, cur, titles) {
            return handled(text, cursor);
        }
    }

    // 2. Continue lists on Enter.
    if key == "\n" {
        if let Some((text, cursor)) = continue_list(content, cur) {
            return handled(text, cursor);
        }
    }

    // 3. Auto-close pairs / wrap selection.
    if let Some((text, cursor)) = auto_pair(key, content, cur, anc) {
        return handled(text, cursor);
    }

    not_handled(content, cursor, anchor)
}

/// Replace the active `[[filter` with `[[Title]]`.
pub fn accept_suggestion(title: &str, content: &str, cursor: i32) -> EditResult {
    let cur = clamp(cursor, content);
    match replace_wikilink(content, cur, title) {
        Some((text, cursor)) => handled(text, cursor),
        None => not_handled(content, cursor, cursor),
    }
}

fn accept_wikilink(content: &str, cursor: usize, titles: &[String]) -> Option<(String, usize)> {
    let filter = wikilink_query(content, cursor)?.to_lowercase();
    let title = titles
        .iter()
        .filter(|t| filter.is_empty() || t.to_lowercase().contains(&filter))
        .min_by_key(|t| t.to_lowercase())?;
    replace_wikilink(content, cursor, title)
}

fn replace_wikilink(content: &str, cursor: usize, title: &str) -> Option<(String, usize)> {
    let open = wikilink_start(content, cursor)?;
    let replacement = format!("[[{}]]", title);
    let mut suffix_start = cursor;
    if content[cursor..].starts_with("]]") {
        suffix_start += 2;
    }
    let new_text = format!(
        "{}{}{}",
        &content[..open],
        replacement,
        &content[suffix_start..]
    );
    Some((new_text, open + replacement.len()))
}

fn continue_list(content: &str, cursor: usize) -> Option<(String, usize)> {
    let line_start = content[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = content[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(content.len());
    // Only continue when the cursor is at the end of the line.
    if cursor != line_end {
        return None;
    }

    let line = &content[line_start..cursor];
    let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
    let indent = &line[..indent_len];
    let rest = &line[indent_len..];

    let (new_marker, body): (String, &str) = if let Some(after) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        if let Some(body) = after.strip_prefix("[ ] ") {
            ("- [ ] ".to_owned(), body)
        } else if let Some(body) = after
            .strip_prefix("[x] ")
            .or_else(|| after.strip_prefix("[X] "))
        {
            ("- [ ] ".to_owned(), body)
        } else {
            ("- ".to_owned(), after)
        }
    } else {
        let dot = rest.find(". ")?;
        let number = &rest[..dot];
        if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let next = number.parse::<u64>().unwrap_or(1) + 1;
        (format!("{next}. "), &rest[dot + 2..])
    };

    if body.trim().is_empty() {
        // Empty item: drop the marker and leave the list.
        let new_text = format!("{}{}", &content[..line_start], &content[cursor..]);
        Some((new_text, line_start))
    } else {
        let insertion = format!("\n{indent}{new_marker}");
        let new_text = format!("{}{}{}", &content[..cursor], insertion, &content[cursor..]);
        Some((new_text, cursor + insertion.len()))
    }
}

fn auto_pair(key: &str, content: &str, cursor: usize, anchor: usize) -> Option<(String, usize)> {
    // Closing brackets are typed over so the auto-inserted pair isn't doubled.
    if key == "]" {
        if content[cursor..].starts_with(']') {
            return Some((content.to_owned(), cursor + 1));
        }
        return None;
    }

    // Wikilinks: the first `[` is left alone, the second one completes `[[…]]`.
    if key == "[" {
        if cursor != anchor {
            let (start, end) = if cursor < anchor {
                (cursor, anchor)
            } else {
                (anchor, cursor)
            };
            let selected = &content[start..end];
            let new_text = format!("{}[[{}]]{}", &content[..start], selected, &content[end..]);
            return Some((new_text, end + 4));
        }
        if !content[..cursor].ends_with('[') {
            return None;
        }
        let insertion = "[]]";
        let new_text = format!("{}{}{}", &content[..cursor], insertion, &content[cursor..]);
        return Some((new_text, cursor + 1));
    }

    let (open, close): (&str, &str) = match key {
        "*" => ("**", "**"),
        "_" => ("_", "_"),
        "`" => ("`", "`"),
        _ => return None,
    };

    // With a selection, wrap it instead of inserting an empty pair.
    if cursor != anchor {
        let (start, end) = if cursor < anchor {
            (cursor, anchor)
        } else {
            (anchor, cursor)
        };
        let selected = &content[start..end];
        let new_text = format!(
            "{}{}{}{}{}",
            &content[..start],
            open,
            selected,
            close,
            &content[end..]
        );
        let new_cursor = end + open.len() + close.len();
        return Some((new_text, new_cursor));
    }

    // Type over an already auto-inserted closer.
    if key == "*" {
        if content[cursor..].starts_with("**") {
            return Some((content.to_owned(), cursor + 2));
        }
    } else if content[cursor..].starts_with(close) {
        return Some((content.to_owned(), cursor + close.len()));
    }

    let insertion = format!("{open}{close}");
    let new_text = format!("{}{}{}", &content[..cursor], insertion, &content[cursor..]);
    Some((new_text, cursor + open.len()))
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn text(res: &EditResult) -> &str {
        res.text.as_str()
    }

    #[test]
    fn continues_unordered_list() {
        let res = handle_key("\n", "- item", 6, 6, &[]);
        assert!(res.handled);
        assert_eq!(text(&res), "- item\n- ");
        assert_eq!(res.cursor, 9);
    }

    #[test]
    fn continues_ordered_list() {
        let res = handle_key("\n", "3. third", 8, 8, &[]);
        assert_eq!(text(&res), "3. third\n4. ");
    }

    #[test]
    fn continues_task_list() {
        let res = handle_key("\n", "- [x] done", 10, 10, &[]);
        assert_eq!(text(&res), "- [x] done\n- [ ] ");
    }

    #[test]
    fn empty_item_exits_list() {
        let res = handle_key("\n", "- ", 2, 2, &[]);
        assert!(res.handled);
        assert_eq!(text(&res), "");
        assert_eq!(res.cursor, 0);
    }

    #[test]
    fn plain_enter_is_not_handled() {
        let res = handle_key("\n", "hello", 5, 5, &[]);
        assert!(!res.handled);
    }

    #[test]
    fn auto_closes_backtick() {
        let res = handle_key("`", "x", 1, 1, &[]);
        assert_eq!(text(&res), "x``");
        assert_eq!(res.cursor, 2);
    }

    #[test]
    fn types_over_closing_backtick() {
        let res = handle_key("`", "``", 1, 1, &[]);
        assert!(res.handled);
        assert_eq!(text(&res), "``");
        assert_eq!(res.cursor, 2);
    }

    #[test]
    fn wraps_selection_in_bold() {
        let res = handle_key("*", "hello", 1, 4, &[]);
        assert_eq!(text(&res), "h**ell**o");
    }

    #[test]
    fn completes_wikilink() {
        let res = handle_key("[", "[", 1, 1, &[]);
        assert_eq!(text(&res), "[[]]");
        assert_eq!(res.cursor, 2);
    }

    #[test]
    fn accepts_wikilink_with_tab() {
        let titles = vec!["Note".to_owned(), "Other".to_owned()];
        let res = handle_key("\t", "[[No]]", 4, 4, &titles);
        assert!(res.handled);
        assert_eq!(text(&res), "[[Note]]");
        assert_eq!(res.cursor, 8);
    }

    #[test]
    fn suggestions_filter_live() {
        let titles = vec!["Alpha".to_owned(), "Beta".to_owned()];
        let (s, show) = suggestions("[[al", 4, &titles);
        assert!(show);
        assert_eq!(s, vec!["Alpha".to_owned()]);
    }
}
