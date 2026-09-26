//! Graph layout — only compiled when the `graph` feature is enabled.
//!
//! Build with: `cargo build --release -p rustidian-ui --features graph`

#[cfg(feature = "graph")]
use crate::links::LinkIndex;

/// 2-D position of a node after layout.
#[cfg(feature = "graph")]
#[derive(Debug, Clone)]
pub struct NodePosition {
    pub id: String,
    pub x: f32,
    pub y: f32,
}

/// Compute a Fruchterman-Reingold force-directed layout for all notes in the
/// index.
///
/// This is called **once** when the graph view is opened, not on every frame.
/// The result is a flat list of (id, x, y) positions ready to be drawn.
#[cfg(feature = "graph")]
pub fn layout(index: &LinkIndex, width: f32, height: f32) -> Vec<NodePosition> {
    use std::collections::HashMap;

    let nodes: Vec<String> = index.outgoing.keys().cloned().collect();
    let n = nodes.len();
    if n == 0 {
        return vec![];
    }

    let node_idx: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();

    // Initial positions: evenly spread on a circle.
    let cx = width / 2.0;
    let cy = height / 2.0;
    let radius = width.min(height) * 0.4;

    let mut pos: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            let angle = 2.0 * std::f32::consts::PI * i as f32 / n as f32;
            (cx + radius * angle.cos(), cy + radius * angle.sin())
        })
        .collect();

    // Fruchterman-Reingold constants.
    let area = width * height;
    let k = (area / n as f32).sqrt();
    let iterations = 50_u32;

    let mut temp = width / 10.0;
    let cooling = temp / (iterations as f32 + 1.0);

    for _ in 0..iterations {
        // Repulsive forces between every pair.
        let mut disp: Vec<(f32, f32)> = vec![(0.0, 0.0); n];
        for v in 0..n {
            for u in 0..n {
                if u == v {
                    continue;
                }
                let dx = pos[v].0 - pos[u].0;
                let dy = pos[v].1 - pos[u].1;
                let dist = (dx * dx + dy * dy).sqrt().max(0.01);
                let force = k * k / dist;
                disp[v].0 += (dx / dist) * force;
                disp[v].1 += (dy / dist) * force;
            }
        }

        // Attractive forces along edges.
        for (source, targets) in &index.outgoing {
            let Some(&si) = node_idx.get(source.as_str()) else {
                continue;
            };
            for target in targets {
                let Some(&ti) = node_idx.get(target.as_str()) else {
                    continue;
                };
                let dx = pos[si].0 - pos[ti].0;
                let dy = pos[si].1 - pos[ti].1;
                let dist = (dx * dx + dy * dy).sqrt().max(0.01);
                let force = dist * dist / k;
                let fx = (dx / dist) * force;
                let fy = (dy / dist) * force;
                disp[si].0 -= fx;
                disp[si].1 -= fy;
                disp[ti].0 += fx;
                disp[ti].1 += fy;
            }
        }

        // Apply displacement, clamped by temperature, bounded to canvas.
        for v in 0..n {
            let (ddx, ddy) = disp[v];
            let dlen = (ddx * ddx + ddy * ddy).sqrt().max(0.01);
            let clamped = dlen.min(temp);
            pos[v].0 = (pos[v].0 + (ddx / dlen) * clamped).clamp(0.0, width);
            pos[v].1 = (pos[v].1 + (ddy / dlen) * clamped).clamp(0.0, height);
        }

        temp -= cooling;
    }

    nodes
        .into_iter()
        .enumerate()
        .map(|(i, id)| NodePosition {
            id,
            x: pos[i].0,
            y: pos[i].1,
        })
        .collect()
}
