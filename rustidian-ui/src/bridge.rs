use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rustidian_core::markdown::{inlines_to_markdown, single_image, Block, Inline};
use rustidian_core::vault;
use rustidian_core::vault::{FolderNode, NoteId, NoteMeta};
use slint::{Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, StyledText, VecModel};

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
/// missing from the map default to collapsed.
pub fn tree_to_rows(
    tree: &[FolderNode],
    expanded: &HashMap<String, bool>,
    selected_folder: &str,
    descending: bool,
) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    for root in tree {
        flatten_node(root, 0, expanded, selected_folder, descending, &mut rows);
    }
    rows
}

fn flatten_node(
    node: &FolderNode,
    depth: i32,
    expanded: &HashMap<String, bool>,
    selected_folder: &str,
    descending: bool,
    rows: &mut Vec<TreeRow>,
) {
    // Re-sort a shallow copy of the references so the direction can change at
    // runtime without rebuilding the folder tree in the core.
    let mut notes: Vec<&NoteMeta> = node.notes.iter().collect();
    notes.sort_by_key(|n| n.title.to_lowercase());
    if descending {
        notes.reverse();
    }
    for note in notes {
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

    let mut children: Vec<&FolderNode> = node.children.iter().collect();
    children.sort_by_key(|c| c.name.to_lowercase());
    if descending {
        children.reverse();
    }

    for child in children {
        let is_expanded = expanded.get(&child.path).copied().unwrap_or(false);
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
            flatten_node(
                child,
                depth + 1,
                expanded,
                selected_folder,
                descending,
                rows,
            );
        }
    }
}

// ── Markdown preview ─────────────────────────────────────────────────────────

/// Maximum side (px) of the in-memory preview thumbnail.  Keeps decoded images
/// small so a note with several pictures does not blow up RAM; the full-quality
/// image is only decoded when the user opens the zoom modal.
const THUMBNAIL_MAX: u32 = 520;

thread_local! {
    static THUMB_CACHE: RefCell<HashMap<PathBuf, (Option<SystemTime>, Image)>> =
        RefCell::new(HashMap::new());
}

/// Load a downscaled thumbnail of *path*, cached by path + modification time so
/// re-rendering the preview on every keystroke stays cheap.
pub fn load_thumbnail(path: &Path) -> Option<Image> {
    let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    THUMB_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((cached_mtime, image)) = cache.get(path) {
            if *cached_mtime == mtime {
                return Some(image.clone());
            }
        }
        let image = make_thumbnail(path)?;
        cache.insert(path.to_path_buf(), (mtime, image.clone()));
        Some(image)
    })
}

fn is_svg(path: &Path) -> bool {
    path.extension()
        .map(|e| e.eq_ignore_ascii_case("svg"))
        .unwrap_or(false)
}

/// Convert an already-decoded raster image into a Slint image.
fn dynamic_to_image(image: image::DynamicImage) -> Image {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    buffer.make_mut_bytes().copy_from_slice(rgba.as_raw());
    Image::from_rgba8(buffer)
}

fn make_thumbnail(path: &Path) -> Option<Image> {
    // SVG is rendered by Slint (resvg); raster formats by the `image` crate.
    if is_svg(path) {
        return Image::load_from_path(path).ok();
    }
    let decoded = image::open(path).ok()?;
    let decoded = if decoded.width() > THUMBNAIL_MAX || decoded.height() > THUMBNAIL_MAX {
        decoded.thumbnail(THUMBNAIL_MAX, THUMBNAIL_MAX)
    } else {
        decoded
    };
    Some(dynamic_to_image(decoded))
}

/// Load the full-quality image (used by the zoom modal).
pub fn load_full_image(path: &Path) -> Option<Image> {
    if is_svg(path) {
        return Image::load_from_path(path).ok();
    }
    let decoded = image::open(path).ok()?;
    Some(dynamic_to_image(decoded))
}

/// Context needed to resolve relative image URLs while bridging blocks.
struct ImgCtx<'a> {
    vault: &'a Path,
    note_id: &'a str,
    attachments: &'a str,
}

impl ImgCtx<'_> {
    fn resolve(&self, url: &str) -> Option<PathBuf> {
        vault::resolve_image_path(self.vault, self.note_id, url, self.attachments)
    }
}

/// Convert parsed Markdown blocks into the flat [`BlockItem`] model consumed by
/// the preview.  Nested structures (lists, quotes) are flattened using the
/// `indent` field.  `vault`/`note_id`/`attachments` are used to resolve and load
/// local images.
pub fn blocks_to_items(
    blocks: &[Block],
    vault: &Path,
    note_id: &str,
    attachments: &str,
) -> Vec<BlockItem> {
    let ctx = ImgCtx {
        vault,
        note_id,
        attachments,
    };
    let mut items = Vec::new();
    push_blocks(blocks, 0, &ctx, &mut items);
    items
}

fn push_blocks(blocks: &[Block], depth: i32, ctx: &ImgCtx, out: &mut Vec<BlockItem>) {
    for block in blocks {
        match block {
            Block::Heading(level, inlines) => {
                let mut item = base(BlockKind::Heading);
                item.level = *level as i32;
                item.markup = styled(inlines);
                out.push(item);
            }
            Block::Paragraph(inlines) => {
                // A paragraph that is just one image renders as a picture.
                if let Some((alt, url)) = single_image(inlines) {
                    if let Some(path) = ctx.resolve(url).filter(|p| p.is_file()) {
                        let mut item = base(BlockKind::Image);
                        item.alt = alt.into();
                        item.image_path = path.to_string_lossy().into_owned().into();
                        if let Some(image) = load_thumbnail(&path) {
                            item.image = image;
                        }
                        out.push(item);
                        continue;
                    }
                }
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
                    push_list_item(item_blocks, &marker, depth, ctx, out);
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
                            push_blocks(std::slice::from_ref(child), depth + 1, ctx, out);
                        }
                        other => {
                            push_blocks(std::slice::from_ref(other), depth, ctx, out);
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

fn push_list_item(
    blocks: &[Block],
    marker: &str,
    depth: i32,
    ctx: &ImgCtx,
    out: &mut Vec<BlockItem>,
) {
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
                push_blocks(std::slice::from_ref(block), depth + 1, ctx, out);
            }
            other => push_blocks(std::slice::from_ref(other), depth, ctx, out),
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
        image: Image::default(),
        image_path: SharedString::default(),
        alt: SharedString::default(),
    }
}

fn styled(inlines: &[Inline]) -> StyledText {
    StyledText::from_markdown(&inlines_to_markdown(inlines)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustidian_core::markdown::parse_blocks;

    #[test]
    fn existing_image_becomes_image_block() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let png = vault.join("pic.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 0, 0, 255]))
            .save(&png)
            .unwrap();

        let items = blocks_to_items(&parse_blocks("![alt](pic.png)"), vault, "Note.md", "att");
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0].kind, BlockKind::Image));
        assert!(items[0].image_path.as_str().ends_with("pic.png"));
    }

    #[test]
    fn missing_image_stays_a_paragraph() {
        let dir = tempfile::tempdir().unwrap();
        let items = blocks_to_items(
            &parse_blocks("![alt](missing.png)"),
            dir.path(),
            "Note.md",
            "att",
        );
        assert!(matches!(items[0].kind, BlockKind::Paragraph));
    }
}
