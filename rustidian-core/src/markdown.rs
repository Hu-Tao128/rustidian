//! Markdown parsing into a UI-agnostic block tree.
//!
//! Instead of rendering Markdown to HTML we consume the `pulldown-cmark` event
//! stream directly and build our own [`Block`] / [`Inline`] tree.  The UI layer
//! is then free to render every variant with a dedicated component, which is
//! how tables, task lists, code blocks and nested quotes get first-class
//! support without patching HTML output case by case.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Inline (span-level) content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Code(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Link { text: Vec<Inline>, url: String },
    Image { alt: String, url: String },
    SoftBreak,
    HardBreak,
}

/// Block-level content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    TaskList(Vec<(bool, Vec<Inline>)>),
    CodeBlock {
        lang: Option<String>,
        code: String,
    },
    BlockQuote(Vec<Block>),
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    ThematicBreak,
}

// ── parser state ─────────────────────────────────────────────────────────────

enum Frame {
    Root {
        blocks: Vec<Block>,
        pending: Vec<Inline>,
    },
    Quote {
        blocks: Vec<Block>,
        pending: Vec<Inline>,
    },
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
        tasks: Vec<(bool, Vec<Inline>)>,
        pending: Vec<Inline>,
    },
    Item {
        blocks: Vec<Block>,
        pending: Vec<Inline>,
        task: Option<bool>,
    },
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Paragraph {
        inlines: Vec<Inline>,
    },
    Strong {
        inlines: Vec<Inline>,
    },
    Emphasis {
        inlines: Vec<Inline>,
    },
    Strikethrough {
        inlines: Vec<Inline>,
    },
    Link {
        url: String,
        inlines: Vec<Inline>,
    },
    Image {
        url: String,
        alt: Vec<Inline>,
    },
    CodeBlock {
        lang: Option<String>,
        code: String,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
        current_row: Vec<String>,
        current_cell: String,
        in_head: bool,
        in_cell: bool,
    },
}

/// Parse *markdown* into a flat list of top-level [`Block`]s.
pub fn parse_blocks(markdown: &str) -> Vec<Block> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(markdown, opts);
    let mut stack: Vec<Frame> = vec![Frame::Root {
        blocks: Vec::new(),
        pending: Vec::new(),
    }];

    for event in parser {
        match event {
            Event::Start(tag) => start_tag(&mut stack, tag),
            Event::End(tag) => end_tag(&mut stack, tag),
            Event::Text(text) => handle_text(&mut stack, &text),
            Event::Code(code) => handle_code(&mut stack, &code),
            Event::SoftBreak => push_inline(&mut stack, Inline::SoftBreak),
            Event::HardBreak => push_inline(&mut stack, Inline::HardBreak),
            Event::Rule => push_block(&mut stack, Block::ThematicBreak),
            Event::TaskListMarker(checked) => {
                if let Some(Frame::Item { task, .. }) = stack.last_mut() {
                    *task = Some(checked);
                }
            }
            // Raw HTML, footnotes, math and definition lists are not rendered.
            _ => {}
        }
    }

    let mut blocks = match stack.pop() {
        Some(Frame::Root {
            mut blocks,
            mut pending,
        }) => {
            if !pending.is_empty() {
                blocks.push(Block::Paragraph(std::mem::take(&mut pending)));
            }
            blocks
        }
        _ => Vec::new(),
    };
    rewrite_blocks(&mut blocks);
    blocks
}

fn start_tag(stack: &mut Vec<Frame>, tag: Tag<'_>) {
    match tag {
        Tag::Paragraph => stack.push(Frame::Paragraph {
            inlines: Vec::new(),
        }),
        Tag::Heading { level, .. } => stack.push(Frame::Heading {
            level: level as u8,
            inlines: Vec::new(),
        }),
        Tag::BlockQuote(_) => stack.push(Frame::Quote {
            blocks: Vec::new(),
            pending: Vec::new(),
        }),
        Tag::CodeBlock(kind) => {
            let lang = match kind {
                CodeBlockKind::Fenced(lang) if !lang.is_empty() => Some(lang.to_string()),
                _ => None,
            };
            stack.push(Frame::CodeBlock {
                lang,
                code: String::new(),
            });
        }
        Tag::List(first) => stack.push(Frame::List {
            ordered: first.is_some(),
            items: Vec::new(),
            tasks: Vec::new(),
            pending: Vec::new(),
        }),
        Tag::Item => stack.push(Frame::Item {
            blocks: Vec::new(),
            pending: Vec::new(),
            task: None,
        }),
        Tag::Table(_) => stack.push(Frame::Table {
            headers: Vec::new(),
            rows: Vec::new(),
            current_row: Vec::new(),
            current_cell: String::new(),
            in_head: false,
            in_cell: false,
        }),
        Tag::TableHead => {
            if let Some(Frame::Table { in_head, .. }) = stack.last_mut() {
                *in_head = true;
            }
        }
        Tag::TableRow => {
            if let Some(Frame::Table { current_row, .. }) = stack.last_mut() {
                current_row.clear();
            }
        }
        Tag::TableCell => {
            if let Some(Frame::Table {
                current_cell,
                in_cell,
                ..
            }) = stack.last_mut()
            {
                current_cell.clear();
                *in_cell = true;
            }
        }
        Tag::Strong => stack.push(Frame::Strong {
            inlines: Vec::new(),
        }),
        Tag::Emphasis => stack.push(Frame::Emphasis {
            inlines: Vec::new(),
        }),
        Tag::Strikethrough => stack.push(Frame::Strikethrough {
            inlines: Vec::new(),
        }),
        Tag::Link { dest_url, .. } => stack.push(Frame::Link {
            url: dest_url.to_string(),
            inlines: Vec::new(),
        }),
        Tag::Image { dest_url, .. } => stack.push(Frame::Image {
            url: dest_url.to_string(),
            alt: Vec::new(),
        }),
        _ => {}
    }
}

fn end_tag(stack: &mut Vec<Frame>, tag: TagEnd) {
    match tag {
        TagEnd::Paragraph => {
            if let Some(Frame::Paragraph { inlines }) = stack.pop() {
                push_block(stack, Block::Paragraph(inlines));
            }
        }
        TagEnd::Heading(_) => {
            if let Some(Frame::Heading { level, inlines }) = stack.pop() {
                push_block(stack, Block::Heading(level, inlines));
            }
        }
        TagEnd::BlockQuote(_) => {
            if let Some(Frame::Quote {
                mut blocks,
                mut pending,
            }) = stack.pop()
            {
                if !pending.is_empty() {
                    blocks.push(Block::Paragraph(std::mem::take(&mut pending)));
                }
                push_block(stack, Block::BlockQuote(blocks));
            }
        }
        TagEnd::CodeBlock => {
            if let Some(Frame::CodeBlock { lang, code }) = stack.pop() {
                push_block(
                    stack,
                    Block::CodeBlock {
                        lang,
                        code: code.trim_end_matches('\n').to_owned(),
                    },
                );
            }
        }
        TagEnd::List(_) => {
            if let Some(Frame::List {
                ordered,
                items,
                tasks,
                ..
            }) = stack.pop()
            {
                if tasks.is_empty() {
                    push_block(stack, Block::List { ordered, items });
                } else {
                    push_block(stack, Block::TaskList(tasks));
                }
            }
        }
        TagEnd::Item => {
            if let Some(Frame::Item {
                mut blocks,
                mut pending,
                task,
            }) = stack.pop()
            {
                if !pending.is_empty() {
                    blocks.push(Block::Paragraph(std::mem::take(&mut pending)));
                }
                match task {
                    Some(checked) => {
                        let inlines = extract_inlines(&blocks);
                        if let Some(Frame::List { tasks, .. }) = stack.last_mut() {
                            tasks.push((checked, inlines));
                        }
                    }
                    None => {
                        if let Some(Frame::List { items, .. }) = stack.last_mut() {
                            items.push(blocks);
                        }
                    }
                }
            }
        }
        TagEnd::Strong => {
            if let Some(Frame::Strong { inlines }) = stack.pop() {
                push_inline(stack, Inline::Strong(inlines));
            }
        }
        TagEnd::Emphasis => {
            if let Some(Frame::Emphasis { inlines }) = stack.pop() {
                push_inline(stack, Inline::Emphasis(inlines));
            }
        }
        TagEnd::Strikethrough => {
            if let Some(Frame::Strikethrough { inlines }) = stack.pop() {
                push_inline(stack, Inline::Strikethrough(inlines));
            }
        }
        TagEnd::Link => {
            if let Some(Frame::Link { url, inlines }) = stack.pop() {
                push_inline(stack, Inline::Link { text: inlines, url });
            }
        }
        TagEnd::Image => {
            if let Some(Frame::Image { url, alt }) = stack.pop() {
                push_inline(
                    stack,
                    Inline::Image {
                        alt: inlines_to_text(&alt),
                        url,
                    },
                );
            }
        }
        TagEnd::TableCell => {
            if let Some(Frame::Table {
                in_head,
                in_cell,
                current_cell,
                headers,
                current_row,
                ..
            }) = stack.last_mut()
            {
                let cell = std::mem::take(current_cell);
                *in_cell = false;
                if *in_head {
                    headers.push(cell);
                } else {
                    current_row.push(cell);
                }
            }
        }
        TagEnd::TableHead => {
            if let Some(Frame::Table { in_head, .. }) = stack.last_mut() {
                *in_head = false;
            }
        }
        TagEnd::TableRow => {
            if let Some(Frame::Table {
                current_row, rows, ..
            }) = stack.last_mut()
            {
                rows.push(std::mem::take(current_row));
            }
        }
        TagEnd::Table => {
            if let Some(Frame::Table { headers, rows, .. }) = stack.pop() {
                push_block(stack, Block::Table { headers, rows });
            }
        }
        _ => {}
    }
}

fn handle_text(stack: &mut [Frame], text: &str) {
    if matches!(stack.last(), Some(Frame::CodeBlock { .. })) {
        if let Some(Frame::CodeBlock { code, .. }) = stack.last_mut() {
            code.push_str(text);
        }
        return;
    }
    if matches!(stack.last(), Some(Frame::Table { in_cell: true, .. })) {
        if let Some(Frame::Table { current_cell, .. }) = stack.last_mut() {
            current_cell.push_str(text);
        }
        return;
    }
    push_inline(stack, Inline::Text(text.to_owned()));
}

fn handle_code(stack: &mut [Frame], code: &str) {
    if matches!(stack.last(), Some(Frame::Table { in_cell: true, .. })) {
        if let Some(Frame::Table { current_cell, .. }) = stack.last_mut() {
            current_cell.push('`');
            current_cell.push_str(code);
            current_cell.push('`');
        }
        return;
    }
    push_inline(stack, Inline::Code(code.to_owned()));
}

/// Attach a finished block to the enclosing block container.
fn push_block(stack: &mut [Frame], block: Block) {
    match stack.last_mut() {
        Some(Frame::Root { blocks, pending })
        | Some(Frame::Quote { blocks, pending })
        | Some(Frame::Item {
            blocks, pending, ..
        }) => {
            if !pending.is_empty() {
                blocks.push(Block::Paragraph(std::mem::take(pending)));
            }
            blocks.push(block);
        }
        _ => {}
    }
}

/// Attach a finished inline span to the enclosing container.
fn push_inline(stack: &mut [Frame], inline: Inline) {
    match stack.last_mut() {
        Some(Frame::Heading { inlines, .. })
        | Some(Frame::Paragraph { inlines })
        | Some(Frame::Strong { inlines })
        | Some(Frame::Emphasis { inlines })
        | Some(Frame::Strikethrough { inlines })
        | Some(Frame::Link { inlines, .. }) => inlines.push(inline),
        Some(Frame::Image { alt, .. }) => alt.push(inline),
        Some(Frame::Root { pending, .. })
        | Some(Frame::Quote { pending, .. })
        | Some(Frame::Item { pending, .. })
        | Some(Frame::List { pending, .. }) => pending.push(inline),
        _ => {}
    }
}

/// Flatten the inline content of a task-list item (usually a single paragraph).
fn extract_inlines(blocks: &[Block]) -> Vec<Inline> {
    if let [Block::Paragraph(inlines)] = blocks {
        return inlines.clone();
    }
    let mut out = Vec::new();
    for block in blocks {
        match block {
            Block::Paragraph(inlines) | Block::Heading(_, inlines) => out.extend(inlines.clone()),
            Block::CodeBlock { code, .. } => out.push(Inline::Code(code.clone())),
            _ => {}
        }
    }
    out
}

/// Reduce inline spans to their plain text (used for image alt text).
fn inlines_to_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(text) | Inline::Code(text) => out.push_str(text),
            Inline::Strong(v) | Inline::Emphasis(v) | Inline::Strikethrough(v) => {
                out.push_str(&inlines_to_text(v));
            }
            Inline::Link { text, .. } => out.push_str(&inlines_to_text(text)),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::SoftBreak | Inline::HardBreak => out.push(' '),
        }
    }
    out
}

// ── wikilinks ────────────────────────────────────────────────────────────────

/// URL scheme used to turn `[[wikilinks]]` into clickable links in the preview.
pub const WIKILINK_SCHEME: &str = "rustidian://";

/// Percent-encode a wikilink target so it survives as a Markdown link URL
/// (spaces and parentheses would otherwise break parsing).
pub fn encode_wikilink_target(target: &str) -> String {
    let mut out = String::with_capacity(target.len());
    for byte in target.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Decode the target encoded in a `rustidian://` URL, or `None` if *url* is not
/// a wikilink URL.
pub fn decode_wikilink_target(url: &str) -> Option<String> {
    let rest = url.strip_prefix(WIKILINK_SCHEME)?;
    Some(percent_decode(rest))
}

/// Percent-decode a URL or URL path (`%20` → space, `%C3%A9` → `é`).  Invalid
/// escapes are left untouched.
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// If *inlines* is effectively a single image (only whitespace/breaks around
/// it), return its `(alt, url)`.
pub fn single_image(inlines: &[Inline]) -> Option<(&str, &str)> {
    let mut found: Option<(&str, &str)> = None;
    for inline in inlines {
        match inline {
            Inline::Image { alt, url } => {
                if found.is_some() {
                    return None;
                }
                found = Some((alt.as_str(), url.as_str()));
            }
            Inline::Text(text) if text.trim().is_empty() => {}
            Inline::SoftBreak | Inline::HardBreak => {}
            _ => return None,
        }
    }
    found
}

/// Walk the block tree and turn `[[wikilinks]]` found in text into links.
fn rewrite_blocks(blocks: &mut [Block]) {
    for block in blocks {
        match block {
            Block::Heading(_, inlines) | Block::Paragraph(inlines) => rewrite_inlines(inlines),
            Block::List { items, .. } => {
                for item in items {
                    rewrite_blocks(item);
                }
            }
            Block::TaskList(tasks) => {
                for (_, inlines) in tasks {
                    rewrite_inlines(inlines);
                }
            }
            Block::BlockQuote(inner) => rewrite_blocks(inner),
            Block::CodeBlock { .. } | Block::Table { .. } | Block::ThematicBreak => {}
        }
    }
}

/// Merge adjacent text spans (pulldown-cmark may split `[[` / `Note` / `]]`)
/// and split wikilinks out of them.
fn rewrite_inlines(inlines: &mut Vec<Inline>) {
    for inline in inlines.iter_mut() {
        match inline {
            Inline::Strong(v) | Inline::Emphasis(v) | Inline::Strikethrough(v) => {
                rewrite_inlines(v);
            }
            Inline::Link { text, .. } => rewrite_inlines(text),
            _ => {}
        }
    }

    let old = std::mem::take(inlines);
    let mut merged: Vec<Inline> = Vec::new();
    let mut buffer = String::new();
    for inline in old {
        match inline {
            Inline::Text(text) => buffer.push_str(&text),
            other => {
                if !buffer.is_empty() {
                    merged.push(Inline::Text(std::mem::take(&mut buffer)));
                }
                merged.push(other);
            }
        }
    }
    if !buffer.is_empty() {
        merged.push(Inline::Text(buffer));
    }

    let mut result = Vec::new();
    for inline in merged {
        match inline {
            Inline::Text(text) => split_wikilinks(&text, &mut result),
            other => result.push(other),
        }
    }
    *inlines = result;
}

/// Split `[[Target]]` / `[[Target|Alias]]` out of a plain text span.
fn split_wikilinks(text: &str, out: &mut Vec<Inline>) {
    let bytes = text.as_bytes();
    let mut last = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some(close) = text[i + 2..].find("]]") {
                if let Some((target, alias)) = parse_wikilink(&text[i + 2..i + 2 + close]) {
                    if last < i {
                        out.push(Inline::Text(text[last..i].to_owned()));
                    }
                    out.push(Inline::Link {
                        text: vec![Inline::Text(alias)],
                        url: format!("{WIKILINK_SCHEME}{}", encode_wikilink_target(&target)),
                    });
                    i = i + 2 + close + 2;
                    last = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    if last < text.len() {
        out.push(Inline::Text(text[last..].to_owned()));
    }
}

/// Parse the inside of a `[[...]]` into `(target, alias)`.
fn parse_wikilink(inner: &str) -> Option<(String, String)> {
    let inner = inner.trim();
    if inner.is_empty() {
        return None;
    }
    let (target, alias) = match inner.split_once('|') {
        Some((target, alias)) => (target.trim(), alias.trim()),
        None => (inner, inner),
    };
    if target.is_empty() {
        return None;
    }
    Some((target.to_owned(), alias.to_owned()))
}

// ── serialisation for the Slint `StyledText` element ─────────────────────────

/// Escape characters that would otherwise be interpreted as Markdown markup.
fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '*' | '_' | '`' | '[' | ']' | '\\' | '~') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Serialise inline spans to a CommonMark subset understood by Slint's
/// `StyledText` element (bold, italic, strikethrough, inline code and links).
pub fn inlines_to_markdown(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(text) => out.push_str(&escape_markdown(text)),
            Inline::Code(code) => {
                out.push('`');
                out.push_str(code);
                out.push('`');
            }
            Inline::Strong(v) => {
                out.push_str("**");
                out.push_str(&inlines_to_markdown(v));
                out.push_str("**");
            }
            Inline::Emphasis(v) => {
                out.push('*');
                out.push_str(&inlines_to_markdown(v));
                out.push('*');
            }
            Inline::Strikethrough(v) => {
                out.push_str("~~");
                out.push_str(&inlines_to_markdown(v));
                out.push_str("~~");
            }
            Inline::Link { text, url } => {
                out.push('[');
                out.push_str(&inlines_to_markdown(text));
                out.push_str("](");
                out.push_str(url);
                out.push(')');
            }
            Inline::Image { alt, url } => {
                // `StyledText` has no image support; surface the alt text as a
                // labelled link so it is at least visible and clickable.
                out.push_str("[🖼 ");
                out.push_str(&escape_markdown(alt));
                out.push_str("](");
                out.push_str(url);
                out.push(')');
            }
            Inline::SoftBreak => out.push('\n'),
            Inline::HardBreak => out.push_str("  \n"),
        }
    }
    out
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn first_paragraph(blocks: &[Block]) -> &[Inline] {
        match &blocks[0] {
            Block::Paragraph(inlines) => inlines,
            other => panic!("expected paragraph, got {other:?}"),
        }
    }

    #[test]
    fn empty_input_returns_no_blocks() {
        assert!(parse_blocks("").is_empty());
    }

    #[test]
    fn heading_parsed() {
        let blocks = parse_blocks("# Hello");
        assert_eq!(
            blocks,
            vec![Block::Heading(1, vec![Inline::Text("Hello".into())])]
        );
    }

    #[test]
    fn bold_italic_strike_parsed() {
        let blocks = parse_blocks("**bold** *italic* ~~strike~~");
        assert_eq!(
            first_paragraph(&blocks),
            &[
                Inline::Strong(vec![Inline::Text("bold".into())]),
                Inline::Text(" ".into()),
                Inline::Emphasis(vec![Inline::Text("italic".into())]),
                Inline::Text(" ".into()),
                Inline::Strikethrough(vec![Inline::Text("strike".into())]),
            ]
        );
    }

    #[test]
    fn inline_code_and_link_parsed() {
        let blocks = parse_blocks("use `cargo` and [docs](https://x.dev)");
        let inlines = first_paragraph(&blocks);
        assert!(inlines.contains(&Inline::Code("cargo".into())));
        assert!(inlines.contains(&Inline::Link {
            text: vec![Inline::Text("docs".into())],
            url: "https://x.dev".into(),
        }));
    }

    #[test]
    fn image_parsed() {
        let blocks = parse_blocks("![alt text](img.png)");
        assert_eq!(
            first_paragraph(&blocks),
            &[Inline::Image {
                alt: "alt text".into(),
                url: "img.png".into(),
            }]
        );
    }

    #[test]
    fn thematic_break_parsed() {
        assert_eq!(parse_blocks("---"), vec![Block::ThematicBreak]);
    }

    #[test]
    fn unordered_list_parsed() {
        let blocks = parse_blocks("- a\n- b");
        match &blocks[0] {
            Block::List { ordered, items } => {
                assert!(!ordered);
                assert_eq!(items.len(), 2);
                assert_eq!(
                    items[0],
                    vec![Block::Paragraph(vec![Inline::Text("a".into())])]
                );
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn ordered_list_parsed() {
        let blocks = parse_blocks("1. first\n2. second");
        match &blocks[0] {
            Block::List { ordered, items } => {
                assert!(ordered);
                assert_eq!(items.len(), 2);
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn nested_list_parsed() {
        let blocks = parse_blocks("- a\n  - b");
        match &blocks[0] {
            Block::List { items, .. } => {
                assert_eq!(items.len(), 1);
                assert!(items[0].iter().any(|b| matches!(b, Block::List { .. })));
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn task_list_parsed() {
        let blocks = parse_blocks("- [ ] todo\n- [x] done");
        match &blocks[0] {
            Block::TaskList(tasks) => {
                assert_eq!(tasks.len(), 2);
                assert!(!tasks[0].0);
                assert_eq!(tasks[0].1, vec![Inline::Text("todo".into())]);
                assert!(tasks[1].0);
                assert_eq!(tasks[1].1, vec![Inline::Text("done".into())]);
            }
            other => panic!("expected task list, got {other:?}"),
        }
    }

    #[test]
    fn fenced_code_block_parsed() {
        let blocks = parse_blocks("```rust\nfn main() {}\n```");
        assert_eq!(
            blocks,
            vec![Block::CodeBlock {
                lang: Some("rust".into()),
                code: "fn main() {}".into(),
            }]
        );
    }

    #[test]
    fn nested_blockquote_parsed() {
        let blocks = parse_blocks("> outer\n> > inner");
        match &blocks[0] {
            Block::BlockQuote(inner) => {
                assert!(inner.iter().any(|b| matches!(b, Block::BlockQuote(_))));
            }
            other => panic!("expected blockquote, got {other:?}"),
        }
    }

    #[test]
    fn table_parsed() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n";
        let blocks = parse_blocks(md);
        match &blocks[0] {
            Block::Table { headers, rows } => {
                assert_eq!(headers, &vec!["A".to_owned(), "B".to_owned()]);
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0], vec!["1".to_owned(), "2".to_owned()]);
                assert_eq!(rows[1], vec!["3".to_owned(), "4".to_owned()]);
            }
            other => panic!("expected table, got {other:?}"),
        }
    }

    #[test]
    fn inlines_to_markdown_keeps_styles() {
        let blocks = parse_blocks("**b** _i_ `c` [l](u)");
        let markup = inlines_to_markdown(first_paragraph(&blocks));
        assert!(markup.contains("**b**"));
        assert!(markup.contains("*i*"));
        assert!(markup.contains("`c`"));
        assert!(markup.contains("[l](u)"));
    }

    #[test]
    fn inlines_to_markdown_escapes_plain_text() {
        let blocks = parse_blocks("a*b_c");
        let markup = inlines_to_markdown(first_paragraph(&blocks));
        assert_eq!(markup, "a\\*b\\_c");
    }

    #[test]
    fn wikilink_becomes_clickable_link() {
        let blocks = parse_blocks("see [[Otra Nota|alias]] and [[Simple]]");
        let inlines = first_paragraph(&blocks);
        match &inlines[1] {
            Inline::Link { text, url } => {
                assert_eq!(text, &vec![Inline::Text("alias".into())]);
                assert_eq!(url, "rustidian://Otra%20Nota");
            }
            other => panic!("expected link, got {other:?}"),
        }
        match &inlines[3] {
            Inline::Link { text, url } => {
                assert_eq!(text, &vec![Inline::Text("Simple".into())]);
                assert_eq!(url, "rustidian://Simple");
            }
            other => panic!("expected link, got {other:?}"),
        }
    }

    #[test]
    fn wikilink_url_roundtrips() {
        let url = format!(
            "{WIKILINK_SCHEME}{}",
            encode_wikilink_target("Proyectos/Mi Nota")
        );
        assert_eq!(url, "rustidian://Proyectos/Mi%20Nota");
        assert_eq!(
            decode_wikilink_target(&url).as_deref(),
            Some("Proyectos/Mi Nota")
        );
        assert_eq!(decode_wikilink_target("https://x.dev"), None);
    }

    #[test]
    fn wikilink_in_code_is_not_a_link() {
        let blocks = parse_blocks("use `[[Nota]]` here");
        let inlines = first_paragraph(&blocks);
        assert!(!inlines.iter().any(|i| matches!(i, Inline::Link { .. })));
    }

    #[test]
    fn inlines_to_markdown_emits_wikilink_url() {
        let blocks = parse_blocks("[[Simple]]");
        let markup = inlines_to_markdown(first_paragraph(&blocks));
        assert_eq!(markup, "[Simple](rustidian://Simple)");
    }

    #[test]
    fn single_image_only_when_paragraph_is_just_an_image() {
        let blocks = parse_blocks("![alt](img.png)");
        assert_eq!(
            single_image(first_paragraph(&blocks)),
            Some(("alt", "img.png"))
        );

        let blocks = parse_blocks("text ![a](b.png)");
        assert_eq!(single_image(first_paragraph(&blocks)), None);

        let blocks = parse_blocks("![a](b.png) ![c](d.png)");
        assert_eq!(single_image(first_paragraph(&blocks)), None);
    }

    #[test]
    fn percent_decode_handles_spaces_and_invalid_escapes() {
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
        assert_eq!(percent_decode("sin%2"), "sin%2");
    }
}
