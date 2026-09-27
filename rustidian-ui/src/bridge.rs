use std::collections::HashMap;

use rustidian_core::markdown::{inlines_to_markdown, Block, Inline};
use rustidian_core::vault::{FolderNode, NoteId, NoteMeta};
use slint::{ModelRc, SharedString, StyledText, VecModel};

use crate::{BacklinkItem, BlockItem, BlockKind, TreeRow, TreeRowKind};

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

// ── Sidebar tree ─────────────────────────────────────────────────────────────

/// Flatten a folder tree into the sidebar's row model.
///
/// `expanded` stores the persisted expansion state per folder path; folders
/// missing from the map default to expanded.
pub fn tree_to_rows(
    tree: &[FolderNode],
    expanded: &HashMap<String, bool>,
    selected_folder: &str,
) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    for root in tree {
        flatten_node(root, 0, expanded, selected_folder, &mut rows);
    }
    rows
}

fn flatten_node(
    node: &FolderNode,
    depth: i32,
    expanded: &HashMap<String, bool>,
    selected_folder: &str,
    rows: &mut Vec<TreeRow>,
) {
    for note in &node.notes {
        rows.push(TreeRow {
            kind: TreeRowKind::Note,
            depth,
            id: note.id.clone().into(),
            name: note.title.clone().into(),
            folder: node.path.clone().into(),
            expanded: false,
            has_children: false,
            is_selected_folder: false,
        });
    }

    for child in &node.children {
        let is_expanded = expanded.get(&child.path).copied().unwrap_or(true);
        rows.push(TreeRow {
            kind: TreeRowKind::Folder,
            depth,
            id: child.path.clone().into(),
            name: child.name.clone().into(),
            folder: child.path.clone().into(),
            expanded: is_expanded,
            has_children: !child.children.is_empty() || !child.notes.is_empty(),
            is_selected_folder: child.path == selected_folder,
        });
        if is_expanded {
            flatten_node(child, depth + 1, expanded, selected_folder, rows);
        }
    }
}

// ── Markdown preview ─────────────────────────────────────────────────────────

/// Convert parsed Markdown blocks into the flat [`BlockItem`] model consumed by
/// the preview.  Nested structures (lists, quotes) are flattened using the
/// `indent` field.
pub fn blocks_to_items(blocks: &[Block]) -> Vec<BlockItem> {
    let mut items = Vec::new();
    push_blocks(blocks, 0, &mut items);
    items
}

fn push_blocks(blocks: &[Block], depth: i32, out: &mut Vec<BlockItem>) {
    for block in blocks {
        match block {
            Block::Heading(level, inlines) => {
                let mut item = base(BlockKind::Heading);
                item.level = *level as i32;
                item.markup = styled(inlines);
                out.push(item);
            }
            Block::Paragraph(inlines) => {
                let mut item = base(BlockKind::Paragraph);
                item.markup = styled(inlines);
                out.push(item);
            }
            Block::List { ordered, items } => {
                for (index, item_blocks) in items.iter().enumerate() {
                    let marker = if *ordered {
                        format!("{}.", index + 1)
                    } else {
                        "•".to_owned()
                    };
                    push_list_item(item_blocks, &marker, depth, out);
                }
            }
            Block::TaskList(tasks) => {
                for (checked, inlines) in tasks {
                    let mut item = base(BlockKind::TaskItem);
                    item.checked = *checked;
                    item.indent = depth;
                    item.markup = styled(inlines);
                    out.push(item);
                }
            }
            Block::CodeBlock { lang, code } => {
                let mut item = base(BlockKind::Code);
                item.lang = lang.clone().unwrap_or_default().into();
                item.code = code.clone().into();
                out.push(item);
            }
            Block::BlockQuote(inner) => {
                for child in inner {
                    match child {
                        Block::Paragraph(inlines) => {
                            let mut item = base(BlockKind::Quote);
                            item.indent = depth;
                            item.markup = styled(inlines);
                            out.push(item);
                        }
                        Block::BlockQuote(_) | Block::List { .. } | Block::TaskList(_) => {
                            push_blocks(std::slice::from_ref(child), depth + 1, out);
                        }
                        other => {
                            push_blocks(std::slice::from_ref(other), depth, out);
                        }
                    }
                }
            }
            Block::Table { headers, rows } => {
                let mut item = base(BlockKind::Table);
                let mut cells: Vec<SharedString> =
                    headers.iter().cloned().map(SharedString::from).collect();
                for row in rows {
                    cells.extend(row.iter().cloned().map(SharedString::from));
                }
                item.columns = headers.len() as i32;
                item.cells = ModelRc::new(VecModel::from(cells));
                out.push(item);
            }
            Block::ThematicBreak => {
                out.push(base(BlockKind::Rule));
            }
        }
    }
}

fn push_list_item(blocks: &[Block], marker: &str, depth: i32, out: &mut Vec<BlockItem>) {
    let mut first = true;
    for block in blocks {
        match block {
            Block::Paragraph(inlines) => {
                let mut item = base(BlockKind::ListItem);
                item.marker = if first {
                    marker.to_owned()
                } else {
                    String::new()
                }
                .into();
                item.indent = depth;
                item.markup = styled(inlines);
                out.push(item);
            }
            Block::List { .. } | Block::TaskList(_) | Block::BlockQuote(_) => {
                push_blocks(std::slice::from_ref(block), depth + 1, out);
            }
            other => push_blocks(std::slice::from_ref(other), depth, out),
        }
        first = false;
    }
}

fn base(kind: BlockKind) -> BlockItem {
    BlockItem {
        kind,
        level: 0,
        indent: 0,
        markup: StyledText::default(),
        lang: SharedString::default(),
        code: SharedString::default(),
        ordered: false,
        marker: SharedString::default(),
        checked: false,
        cells: ModelRc::default(),
        columns: 0,
    }
}

fn styled(inlines: &[Inline]) -> StyledText {
    StyledText::from_markdown(&inlines_to_markdown(inlines)).unwrap_or_default()
}
