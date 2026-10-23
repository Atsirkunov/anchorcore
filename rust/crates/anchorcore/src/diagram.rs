//! Diagram normal form — structured graphs rendered to deterministic text (R46.1).
//!
//! Every flow source (drawio, excalidraw, vsdx, Miro, …) parses into one
//! `Diagram { frames, nodes, edges }` and renders through `render`, so the
//! output is markdown that `markdown.rs` chunks with breadcrumbs + line
//! ranges unchanged. Rules:
//!
//! - A frame is a `##` heading (nested frames go one level deeper, capped at
//!   `######`); a frame with no content anywhere in its subtree emits nothing.
//! - Nodes render as bullets in reading order: rows top → bottom, then
//!   left → right inside a row. Rows cluster by a y tolerance (half the
//!   median node height), so small coordinate jitter never reorders text.
//! - Edges render as `A → B: label` bullets in the source node's frame
//!   (else the target's, else the root). Endpoints resolve to node text,
//!   falling back to `#id`; a missing endpoint reads `?`. Unlabeled and
//!   dangling edges are kept — only an edge with no endpoints and no label
//!   carries nothing and is dropped.
//! - z-order, style and geometry beyond reading order are ignored.
//!
//! Frame membership is the parser's job (explicit `parent` ids or geometric
//! containment); this module never infers it.

use std::collections::HashMap;

/// Row tolerance when no node carries a height.
const DEFAULT_ROW_TOL: f64 = 20.0;
const MAX_HEADING: usize = 6;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Diagram {
    /// Rendered as the `#` heading (the markdown root title); empty = none.
    pub title: String,
    pub frames: Vec<Frame>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    pub id: String,
    pub title: String,
    /// Enclosing frame id; unknown or cyclic parents read as top-level.
    pub parent: Option<String>,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub id: String,
    pub text: String,
    /// Owning frame id; unknown ids read as the root.
    pub frame: Option<String>,
    pub x: f64,
    pub y: f64,
    /// Height when known (0 = unknown); only feeds the row tolerance.
    pub h: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Edge {
    pub from: Option<String>,
    pub to: Option<String>,
    pub label: String,
}

/// Per-group bodies: index 0 is the root, `i + 1` is `frames[i]`.
#[derive(Debug, Clone, Default)]
struct Group {
    nodes: Vec<String>,
    edges: Vec<String>,
}

struct Ctx<'a> {
    d: &'a Diagram,
    groups: Vec<Group>,
    /// Child frame indices per parent group (0 = top-level), reading order.
    children: Vec<Vec<usize>>,
}

/// Render a diagram to markdown. Deterministic for a given input.
pub fn render(d: &Diagram) -> String {
    let ctx = layout(d);
    let mut out = String::new();
    let title = collapse(&d.title);
    if !title.is_empty() {
        out.push_str(&format!("# {title}\n\n"));
    }
    out.push_str(&group_body(&ctx.groups[0]));
    for &f in &ctx.children[0] {
        out.push_str(&render_frame(&ctx, f, 0));
    }
    let trimmed = out.trim_end();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

fn layout(d: &Diagram) -> Ctx<'_> {
    let frame_ix: HashMap<&str, usize> = d.frames.iter().enumerate().map(|(i, f)| (f.id.as_str(), i)).collect();
    let node_ix: HashMap<&str, usize> = d.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let node_group: Vec<usize> = d
        .nodes
        .iter()
        .map(|n| n.frame.as_deref().and_then(|id| frame_ix.get(id)).map_or(0, |&i| i + 1))
        .collect();
    let tol = row_tolerance(&d.nodes);
    let mut groups = vec![Group::default(); d.frames.len() + 1];
    place_nodes(d, &node_group, tol, &mut groups);
    place_edges(d, &node_ix, &node_group, tol, &mut groups);
    let children = frame_children(d, &frame_ix, tol);
    Ctx { d, groups, children }
}

fn place_nodes(d: &Diagram, node_group: &[usize], tol: f64, groups: &mut [Group]) {
    let pts: Vec<(f64, f64)> = d.nodes.iter().map(|n| (n.x, n.y)).collect();
    for i in reading_order(&pts, tol) {
        let text = collapse(&d.nodes[i].text);
        if !text.is_empty() {
            groups[node_group[i]].nodes.push(format!("- {text}"));
        }
    }
}

/// Edge sort key: (source rank, target rank, label, input index).
type EdgeKey = (usize, usize, String, usize);

fn place_edges<'a>(d: &'a Diagram, node_ix: &HashMap<&str, usize>, node_group: &[usize], tol: f64, groups: &mut [Group]) {
    let pts: Vec<(f64, f64)> = d.nodes.iter().map(|n| (n.x, n.y)).collect();
    let mut rank = vec![0usize; d.nodes.len()];
    for (r, i) in reading_order(&pts, tol).into_iter().enumerate() {
        rank[i] = r;
    }
    let lookup = |id: &'a Option<String>| id.as_deref().filter(|s| !s.is_empty()).map(|s| (s, node_ix.get(s).copied()));
    let mut keyed: Vec<(EdgeKey, usize, String)> = Vec::new();
    for (ei, e) in d.edges.iter().enumerate() {
        let (from, to) = (lookup(&e.from), lookup(&e.to));
        let label = collapse(&e.label);
        if from.is_none() && to.is_none() && label.is_empty() {
            continue;
        }
        let node_of = |p: Option<(&str, Option<usize>)>| p.and_then(|(_, i)| i);
        let group = node_of(from).or(node_of(to)).map_or(0, |i| node_group[i]);
        let rank_of = |p| node_of(p).map_or(usize::MAX, |i| rank[i]);
        let mut line = format!("- {} → {}", endpoint(d, from), endpoint(d, to));
        if !label.is_empty() {
            line.push_str(&format!(": {label}"));
        }
        keyed.push(((rank_of(from), rank_of(to), label, ei), group, line));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, group, line) in keyed {
        groups[group].edges.push(line);
    }
}

/// Node text, else `#id` for a textless or unknown node, else `?`.
fn endpoint(d: &Diagram, p: Option<(&str, Option<usize>)>) -> String {
    match p {
        None => "?".to_string(),
        Some((id, Some(i))) => {
            let text = collapse(&d.nodes[i].text);
            if text.is_empty() {
                format!("#{id}")
            } else {
                text
            }
        }
        Some((id, None)) => format!("#{id}"),
    }
}

fn frame_children(d: &Diagram, frame_ix: &HashMap<&str, usize>, tol: f64) -> Vec<Vec<usize>> {
    let mut children = vec![Vec::new(); d.frames.len() + 1];
    let pts: Vec<(f64, f64)> = d.frames.iter().map(|f| (f.x, f.y)).collect();
    for f in reading_order(&pts, tol) {
        let parent = valid_parent(d, frame_ix, f).map_or(0, |p| p + 1);
        children[parent].push(f);
    }
    children
}

/// The frame's parent index, unless it is unknown, itself, or on a cycle.
fn valid_parent(d: &Diagram, frame_ix: &HashMap<&str, usize>, f: usize) -> Option<usize> {
    let parent_of = |i: usize| d.frames[i].parent.as_deref().and_then(|id| frame_ix.get(id).copied());
    let first = parent_of(f)?;
    let mut cur = Some(first);
    for _ in 0..d.frames.len() {
        match cur {
            None => return Some(first),
            Some(c) if c == f => return None,
            Some(c) => cur = parent_of(c),
        }
    }
    // Walked `len` steps without reaching a root: an ancestor cycle that
    // excludes `f` — `f` still nests under its (top-level) parent.
    Some(first)
}

fn render_frame(ctx: &Ctx<'_>, f: usize, depth: usize) -> String {
    let mut body = group_body(&ctx.groups[f + 1]);
    for &c in &ctx.children[f + 1] {
        body.push_str(&render_frame(ctx, c, depth + 1));
    }
    if body.is_empty() {
        return String::new();
    }
    let title = collapse(&ctx.d.frames[f].title);
    let title = if title.is_empty() { "Untitled frame".to_string() } else { title };
    let hashes = "#".repeat((depth + 2).min(MAX_HEADING));
    format!("{hashes} {title}\n\n{body}")
}

fn group_body(g: &Group) -> String {
    let mut out = String::new();
    for list in [&g.nodes, &g.edges] {
        if !list.is_empty() {
            out.push_str(&list.join("\n"));
            out.push_str("\n\n");
        }
    }
    out
}

/// Half the median known node height, else `DEFAULT_ROW_TOL`.
fn row_tolerance(nodes: &[Node]) -> f64 {
    let mut hs: Vec<f64> = nodes.iter().map(|n| n.h).filter(|h| h.is_finite() && *h > 0.0).collect();
    if hs.is_empty() {
        return DEFAULT_ROW_TOL;
    }
    hs.sort_by(f64::total_cmp);
    hs[hs.len() / 2] / 2.0
}

/// Indices in reading order: rows by y (a row spans `tol` from its top),
/// then x inside a row; input index breaks every remaining tie.
fn reading_order(pts: &[(f64, f64)], tol: f64) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..pts.len()).collect();
    idx.sort_by(|&a, &b| pts[a].1.total_cmp(&pts[b].1).then(a.cmp(&b)));
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut row_top = f64::NEG_INFINITY;
    for i in idx {
        if rows.is_empty() || pts[i].1 - row_top > tol {
            rows.push(Vec::new());
            row_top = pts[i].1;
        }
        rows.last_mut().expect("row pushed above").push(i);
    }
    for row in &mut rows {
        row.sort_by(|&a, &b| pts[a].0.total_cmp(&pts[b].0).then(a.cmp(&b)));
    }
    rows.concat()
}

/// Whitespace (incl. newlines inside a sticky or shape) collapsed to spaces.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, text: &str, frame: Option<&str>, x: f64, y: f64) -> Node {
        Node { id: id.into(), text: text.into(), frame: frame.map(Into::into), x, y, h: 40.0 }
    }

    fn frame(id: &str, title: &str, parent: Option<&str>, x: f64, y: f64) -> Frame {
        Frame { id: id.into(), title: title.into(), parent: parent.map(Into::into), x, y }
    }

    fn edge(from: Option<&str>, to: Option<&str>, label: &str) -> Edge {
        Edge { from: from.map(Into::into), to: to.map(Into::into), label: label.into() }
    }

    fn grid(jitter: impl Fn(usize) -> (f64, f64)) -> Diagram {
        let names = ["A1", "A2", "A3", "B1", "B2", "B3"];
        let nodes = names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let (dx, dy) = jitter(i);
                node(n, n, None, (i % 3) as f64 * 120.0 + dx, (i / 3) as f64 * 100.0 + dy)
            })
            .collect();
        Diagram { nodes, ..Default::default() }
    }

    #[test]
    fn nodes_read_top_left_to_bottom_right() {
        let mut d = grid(|_| (0.0, 0.0));
        d.nodes.reverse();
        assert_eq!(render(&d), "- A1\n- A2\n- A3\n- B1\n- B2\n- B3\n");
    }

    #[test]
    fn reading_order_is_stable_under_jitter() {
        let base = render(&grid(|_| (0.0, 0.0)));
        let jitters: [fn(usize) -> (f64, f64); 3] = [
            |i| (0.0, if i % 2 == 0 { 9.0 } else { -9.0 }),
            |i| ((i as f64 * 7.3) % 11.0 - 5.0, (i as f64 * 3.7) % 13.0 - 6.0),
            |i| (if i == 2 { -15.0 } else { 0.0 }, if i == 1 { 18.0 } else { 0.0 }),
        ];
        for j in jitters {
            assert_eq!(render(&grid(j)), base);
        }
    }

    #[test]
    fn frames_become_headings_and_root_comes_first() {
        let d = Diagram {
            title: "Onboarding board".into(),
            frames: vec![frame("f2", "Week 2", None, 0.0, 500.0), frame("f1", "Week 1", None, 0.0, 0.0)],
            nodes: vec![
                node("n3", "Ship laptop", Some("f2"), 0.0, 520.0),
                node("n1", "Sign contract", Some("f1"), 0.0, 20.0),
                node("n0", "Legend: green = done", None, 900.0, 0.0),
            ],
            edges: vec![edge(Some("n1"), Some("n3"), "then")],
        };
        assert_eq!(
            render(&d),
            "# Onboarding board\n\n- Legend: green = done\n\n## Week 1\n\n- Sign contract\n\n- Sign contract → Ship laptop: then\n\n## Week 2\n\n- Ship laptop\n"
        );
    }

    #[test]
    fn nested_frames_go_deeper_and_cycles_read_top_level() {
        let d = Diagram {
            title: String::new(),
            frames: vec![
                frame("outer", "Outer", None, 0.0, 0.0),
                frame("inner", "Inner", Some("outer"), 10.0, 10.0),
                frame("a", "Cycle A", Some("b"), 0.0, 900.0),
                frame("b", "Cycle B", Some("a"), 0.0, 1200.0),
            ],
            nodes: vec![
                node("x", "Deep", Some("inner"), 20.0, 20.0),
                node("y", "In A", Some("a"), 0.0, 920.0),
                node("z", "In B", Some("b"), 0.0, 1220.0),
            ],
            edges: vec![],
        };
        assert_eq!(
            render(&d),
            "## Outer\n\n### Inner\n\n- Deep\n\n## Cycle A\n\n- In A\n\n## Cycle B\n\n- In B\n"
        );
    }

    #[test]
    fn empty_frames_are_skipped_but_untitled_ones_are_named() {
        let d = Diagram {
            title: String::new(),
            frames: vec![frame("e", "Empty", None, 0.0, 0.0), frame("u", "  ", None, 0.0, 300.0)],
            nodes: vec![node("n", "Lonely note", Some("u"), 0.0, 320.0), node("blank", "   ", Some("e"), 0.0, 10.0)],
            edges: vec![],
        };
        assert_eq!(render(&d), "## Untitled frame\n\n- Lonely note\n");
    }

    #[test]
    fn edges_resolve_text_and_keep_dangling_and_unlabeled() {
        let d = Diagram {
            nodes: vec![node("a", "Request\nreceived", None, 0.0, 0.0), node("b", "", None, 200.0, 0.0)],
            edges: vec![
                edge(Some("a"), Some("b"), ""),
                edge(Some("a"), None, "escalate"),
                edge(Some("ghost"), Some("a"), "retry"),
                edge(None, None, ""),
                edge(Some(""), None, ""),
            ],
            ..Default::default()
        };
        assert_eq!(
            render(&d),
            "- Request received\n\n- Request received → #b\n- Request received → ?: escalate\n- #ghost → Request received: retry\n"
        );
    }

    #[test]
    fn edges_follow_source_frame_then_target_frame() {
        let d = Diagram {
            title: String::new(),
            frames: vec![frame("f", "Review", None, 0.0, 0.0)],
            nodes: vec![node("r", "Review PR", Some("f"), 0.0, 0.0), node("m", "Merge", None, 0.0, 500.0)],
            edges: vec![edge(Some("m"), Some("r"), "reopen"), edge(None, Some("r"), "start")],
        };
        let out = render(&d);
        assert_eq!(out, "- Merge\n\n- Merge → Review PR: reopen\n\n## Review\n\n- Review PR\n\n- ? → Review PR: start\n");
    }

    #[test]
    fn empty_diagram_renders_nothing() {
        assert_eq!(render(&Diagram::default()), "");
        let d = Diagram { frames: vec![frame("f", "Empty", None, 0.0, 0.0)], ..Default::default() };
        assert_eq!(render(&d), "");
    }

    #[test]
    fn markdown_chunker_takes_output_unchanged() {
        let d = Diagram {
            title: "Release flow".into(),
            frames: vec![frame("p", "Prepare", None, 0.0, 0.0), frame("s", "Ship", None, 0.0, 400.0)],
            nodes: vec![
                node("bump", "Bump version", Some("p"), 0.0, 0.0),
                node("tag", "Push tag", Some("s"), 0.0, 400.0),
                node("hash", "# not a heading", Some("s"), 200.0, 400.0),
            ],
            edges: vec![edge(Some("bump"), Some("tag"), "after notes")],
        };
        let (sections, chunks) = crate::markdown::chunk_markdown(&render(&d), "release.drawio");
        let paths: Vec<&str> = sections.iter().map(|s| s.path.as_str()).collect();
        assert!(paths.iter().any(|p| p.ends_with("Prepare")), "{paths:?}");
        assert!(paths.iter().any(|p| p.ends_with("Ship")), "{paths:?}");
        assert!(!paths.iter().any(|p| p.contains("not a heading")), "{paths:?}");
        assert!(paths.iter().all(|p| !p.starts_with(" > ")), "{paths:?}");
        let edge_chunk = chunks.iter().find(|c| c.content.contains("Bump version → Push tag: after notes")).expect("edge line chunked");
        assert!(edge_chunk.path.ends_with("Prepare"), "{}", edge_chunk.path);
        assert!(edge_chunk.line_start > 0);
    }
}
