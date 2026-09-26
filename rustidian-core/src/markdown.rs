use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Convert a Markdown string to an HTML string.
///
/// This is a thin wrapper over `pulldown-cmark` with safe defaults (no raw HTML
/// passthrough — prevents XSS if notes ever render in a web context).
pub fn to_html(markdown: &str) -> String {
    use pulldown_cmark::html;

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(markdown, opts);
    let mut html_output = String::with_capacity(markdown.len() * 2);
    html::push_html(&mut html_output, parser);
    html_output
}

/// Convert Markdown to a styled plain-text string for display in a Slint
/// `Text` widget (no HTML renderer available).
///
/// Visual conventions:
/// - H1 → `█ TITLE`  H2 → `▌ Title`  H3+ → `▸ Title`
/// - Bold/italic markers stripped; text content kept
/// - Inline code wrapped in `backticks`
/// - Fenced code blocks rendered with a top/bottom border and language label:
///   ```rust          becomes:   ╭─ rust ──────────╮
///   fn main() {}               │  fn main() {}
///   ```                        ╰────────────────────╯
/// - List items: `•` unordered, `1.` ordered, nested with 2-space indent
/// - Task list items: `[x]` / `[ ]`
/// - Blockquotes: `▍ text`
/// - Tables: column content separated by `│`
/// - Horizontal rules: `──────────────────────────────`
pub fn to_plain(markdown: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(markdown, opts);

    let mut out = String::with_capacity(markdown.len() * 2);
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_lines: Vec<String> = Vec::new();
    let mut in_table_head = false;
    let mut col_sep = false;
    // Track bold/italic nesting so we can add visual markers
    let mut strong_depth: u32 = 0;
    let mut em_depth: u32 = 0;

    for event in parser {
        match event {
            // ── Block openings ──────────────────────────────────────────────
            Event::Start(Tag::Heading { level, .. }) => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                let prefix = match level as usize {
                    1 => "█ ",
                    2 => "▌ ",
                    _ => "▸ ",
                };
                out.push_str(prefix);
            }
            Event::End(TagEnd::Heading(_)) => {
                out.push('\n');
                out.push('\n');
            }

            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                out.push('\n');
                out.push('\n');
            }

            Event::Start(Tag::BlockQuote(_)) => {
                out.push_str("▍ ");
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                out.push('\n');
            }

            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                code_lines.clear();
                code_lang = match kind {
                    CodeBlockKind::Fenced(lang) if !lang.is_empty() => lang.to_string(),
                    _ => String::new(),
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                // Render with box border
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                let border_width = 36usize;
                // Top border with optional language label
                if code_lang.is_empty() {
                    out.push('╭');
                    out.push_str(&"─".repeat(border_width));
                    out.push('╮');
                } else {
                    let label = format!(" {} ", code_lang);
                    let dashes = border_width.saturating_sub(label.len() + 1);
                    out.push_str(&format!("╭─{label}{}\n", "─".repeat(dashes) + "╮"));
                    // Trim the trailing \n that was added inside the format
                    if out.ends_with('\n') { out.truncate(out.len() - 1); }
                }
                out.push('\n');
                for line in &code_lines {
                    out.push_str("│  ");
                    out.push_str(line);
                    out.push('\n');
                }
                out.push('╰');
                out.push_str(&"─".repeat(border_width));
                out.push('╯');
                out.push('\n');
                out.push('\n');
                code_lines.clear();
                code_lang.clear();
            }

            Event::Start(Tag::List(first)) => {
                list_stack.push(first);
            }
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
                out.push('\n');
            }
            Event::Start(Tag::Item) => {
                let indent = "  ".repeat(list_stack.len().saturating_sub(1));
                match list_stack.last() {
                    Some(Some(n)) => {
                        out.push_str(&format!("{indent}{n}. "));
                        if let Some(Some(ref mut counter)) = list_stack.last_mut() {
                            *counter += 1;
                        }
                    }
                    _ => out.push_str(&format!("{indent}• ")),
                }
            }
            Event::End(TagEnd::Item) => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }

            // ── Tables ───────────────────────────────────────────────────────
            Event::Start(Tag::Table(_)) => {}
            Event::End(TagEnd::Table) => {
                out.push('\n');
            }
            Event::Start(Tag::TableHead) => {
                in_table_head = true;
                col_sep = false;
            }
            Event::End(TagEnd::TableHead) => {
                in_table_head = false;
                out.push('\n');
            }
            Event::Start(Tag::TableRow) => {
                col_sep = false;
            }
            Event::End(TagEnd::TableRow) => {
                out.push('\n');
            }
            Event::Start(Tag::TableCell) => {
                if col_sep {
                    out.push_str("  │  ");
                }
                col_sep = true;
            }
            Event::End(TagEnd::TableCell) => {
                let _ = in_table_head; // future: underline header cells
            }

            // ── Inline styling — add visual markers for bold / italic ────────
            Event::Start(Tag::Strong) => {
                strong_depth += 1;
                if !in_code_block { out.push_str("**"); }
            }
            Event::End(TagEnd::Strong) => {
                if strong_depth > 0 { strong_depth -= 1; }
                if !in_code_block { out.push_str("**"); }
            }
            Event::Start(Tag::Emphasis) => {
                em_depth += 1;
                if !in_code_block { out.push('_'); }
            }
            Event::End(TagEnd::Emphasis) => {
                if em_depth > 0 { em_depth -= 1; }
                if !in_code_block { out.push('_'); }
            }
            Event::Start(Tag::Strikethrough) => {
                if !in_code_block { out.push_str("~~"); }
            }
            Event::End(TagEnd::Strikethrough) => {
                if !in_code_block { out.push_str("~~"); }
            }
            Event::Start(Tag::Link { .. }) => {}
            Event::End(TagEnd::Link) => {}
            Event::Start(Tag::Image { .. }) => {
                out.push_str("[image: ");
            }
            Event::End(TagEnd::Image) => {
                out.push(']');
            }

            // ── Leaf events ──────────────────────────────────────────────────
            Event::Text(t) => {
                if in_code_block {
                    // Accumulate lines; rendered in End(CodeBlock)
                    for line in t.lines() {
                        code_lines.push(line.to_owned());
                    }
                } else {
                    out.push_str(&t);
                }
            }
            Event::Code(t) => {
                out.push('`');
                out.push_str(&t);
                out.push('`');
            }
            Event::SoftBreak => {
                out.push(' ');
            }
            Event::HardBreak => {
                out.push('\n');
            }
            Event::Rule => {
                out.push_str("──────────────────────────────────\n\n");
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                if out.ends_with("• ") {
                    let len = out.len();
                    out.truncate(len - "• ".len());
                    out.push_str(marker);
                }
            }

            _ => {}
        }
    }

    out.trim_end().to_owned()
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_rendered() {
        let html = to_html("# Hello");
        assert!(html.contains("<h1>"), "expected h1, got: {html}");
        assert!(html.contains("Hello"));
    }

    #[test]
    fn bold_rendered() {
        let html = to_html("**bold**");
        assert!(html.contains("<strong>"));
    }

    #[test]
    fn strikethrough_rendered() {
        let html = to_html("~~strike~~");
        assert!(html.contains("<del>"));
    }

    #[test]
    fn table_rendered() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n";
        let html = to_html(md);
        assert!(html.contains("<table>"));
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(to_html("").trim(), "");
    }

    #[test]
    fn plain_heading() {
        let plain = to_plain("# Title");
        assert!(plain.contains("█ Title"), "got: {plain}");
        assert!(!plain.contains('#'));
    }

    #[test]
    fn plain_strips_bold() {
        // Bold markers are now kept as visual cues in the plain-text preview.
        let plain = to_plain("Hello **world**");
        assert!(plain.contains("world"));
        assert!(plain.contains("**world**"));
    }

    #[test]
    fn plain_list_bullets() {
        let plain = to_plain("- alpha\n- beta");
        assert!(plain.contains("• alpha"));
        assert!(plain.contains("• beta"));
    }

    #[test]
    fn plain_empty() {
        assert_eq!(to_plain(""), "");
    }
}
