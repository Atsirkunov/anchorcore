//! Diagram file parsers → `diagram::Diagram` (R46.2): draw.io, Excalidraw, SVG.
//!
//! `extract.rs` dispatches here by kind (`extract::diagram_kind`) and every
//! format renders through `diagram::render`, so the stored text is markdown
//! (`markdown::is_markdown_filename` routes these names to the block chunker).
//!
//! - **draw.io** (`.drawio`/`.dio`, and the `content` attribute of a
//!   `.drawio.svg`): plain or compressed `<diagram>` pages (base64 → raw
//!   deflate → URI-decoded XML). Several pages become top-level frames.
//!   Labeled containers (any vertex that parents other cells: swimlanes,
//!   containers) become frames, unlabeled groups are transparent. Edge label
//!   cells join their edge's label. `html=1` labels are tag-stripped.
//! - **Excalidraw** (`.excalidraw`/`.excalidraw.json`, and the embedded
//!   scene of an `.excalidraw.svg`): `frame` elements are frames, bound text
//!   names its container (shape → node text, arrow → edge label), arrows
//!   and lines bind via `startBinding`/`endBinding`. Deleted elements drop.
//! - **SVG**: the root `<title>` titles the diagram, the root `<desc>` reads
//!   first, every `<text>` (tspans joined) is a node at its `x`/`y`. No edges.
//!   Transforms are ignored — positions only order the text.

use std::collections::{HashMap, HashSet};
use std::io::Read;

use base64::Engine;
use quick_xml::events::{BytesStart, Event};

use crate::diagram::{render, Diagram, Edge, Frame, Node};

/// Cap for one inflated payload (matches the zip-part cap in `extract.rs`).
const MAX_INFLATE: u64 = 64 * 1024 * 1024;
/// Page frames sit this far apart vertically so pages keep document order.
const PAGE_STRIDE: f64 = 1.0e9;

// ----- shared XML helpers -----

fn attr(e: &BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref() == key).map(|a| {
        let raw = a.value.into_owned();
        quick_xml::escape::unescape(&raw).map(|c| c.into_owned()).unwrap_or(raw)
    })
}

fn attr_f64(e: &BytesStart, key: &str) -> f64 {
    // SVG allows coordinate lists (`x="10 20 30"`): the first one places the text.
    attr(e, key)
        .and_then(|v| v.split([' ', ',']).find(|s| !s.is_empty()).and_then(|s| s.parse().ok()))
        .unwrap_or(0.0)
}

fn local_name(e: &BytesStart) -> String {
    e.local_name().as_ref().to_string()
}

fn inflate(bytes: &[u8], zlib: bool) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let res = if zlib {
        flate2::read::ZlibDecoder::new(bytes).take(MAX_INFLATE).read_to_end(&mut out)
    } else {
        flate2::read::DeflateDecoder::new(bytes).take(MAX_INFLATE).read_to_end(&mut out)
    };
    res.map_err(|e| format!("inflate: {e}"))?;
    Ok(out)
}

/// `decodeURIComponent`: `%XX` runs → UTF-8 bytes; malformed escapes stay literal.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = if b[i] == b'%' && i + 2 < b.len() {
            std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        } else {
            None
        };
        match hex {
            Some(v) => {
                out.push(v);
                i += 3;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ----- draw.io -----

#[derive(Debug, Default)]
struct Cell {
    id: String,
    text: String,
    html: bool,
    vertex: bool,
    edge: bool,
    parent: Option<String>,
    source: Option<String>,
    target: Option<String>,
    x: f64,
    y: f64,
    h: f64,
}

#[derive(Debug, Default)]
struct Page {
    name: String,
    cells: Vec<Cell>,
}

/// Parse a draw.io file (`<mxfile>` or a bare `<mxGraphModel>`) to text.
pub fn drawio_to_text(xml: &str) -> Result<String, String> {
    let mut pages = Vec::new();
    parse_drawio(xml, &mut pages, true)?;
    if pages.iter().all(|p| p.cells.is_empty()) && !xml.contains("mxGraphModel") && !xml.contains("<mxfile") {
        return Err("not a draw.io diagram".to_string());
    }
    Ok(render(&drawio_diagram(&pages)))
}

#[derive(Default)]
struct DrawioWalk {
    in_diagram: bool,
    /// `<UserObject>`/`<object>` wrapper: its `id` + `label` own the inner cell.
    wrapper: Option<(String, Option<String>)>,
    open_cell: bool,
}

fn parse_drawio(xml: &str, pages: &mut Vec<Page>, outer: bool) -> Result<(), String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut w = DrawioWalk::default();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => drawio_open(&e, pages, &mut w, outer, false),
            Ok(Event::Empty(e)) => drawio_open(&e, pages, &mut w, outer, true),
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                "diagram" => w.in_diagram = false,
                "mxCell" => w.open_cell = false,
                "UserObject" | "object" => w.wrapper = None,
                _ => {}
            },
            Ok(Event::Text(t)) if w.in_diagram => {
                let payload = t.into_inner().trim().to_string();
                if !payload.is_empty() && pages.last().is_some_and(|p| p.cells.is_empty()) {
                    parse_drawio(&decode_drawio_payload(&payload)?, pages, false)?;
                }
            }
            Ok(Event::Eof) => return Ok(()),
            Err(e) => return Err(format!("xml: {e}")),
            _ => {}
        }
    }
}

fn drawio_open(e: &BytesStart, pages: &mut Vec<Page>, w: &mut DrawioWalk, outer: bool, empty: bool) {
    match e.local_name().as_ref() {
        "diagram" if outer => {
            pages.push(Page { name: attr(e, "name").unwrap_or_default(), cells: Vec::new() });
            w.in_diagram = !empty;
        }
        "UserObject" | "object" if !empty => w.wrapper = Some((attr(e, "id").unwrap_or_default(), attr(e, "label"))),
        "mxCell" => {
            if pages.is_empty() {
                pages.push(Page::default());
            }
            let cell = drawio_cell(e, w.wrapper.as_ref());
            pages.last_mut().expect("page pushed above").cells.push(cell);
            w.open_cell = !empty;
        }
        "mxGeometry" if w.open_cell && attr(e, "as").is_none_or(|a| a == "geometry") => {
            if let Some(c) = pages.last_mut().and_then(|p| p.cells.last_mut()) {
                (c.x, c.y, c.h) = (attr_f64(e, "x"), attr_f64(e, "y"), attr_f64(e, "height"));
            }
        }
        _ => {}
    }
}

fn drawio_cell(e: &BytesStart, wrapper: Option<&(String, Option<String>)>) -> Cell {
    let style = attr(e, "style").unwrap_or_default();
    let id = wrapper.map(|w| w.0.clone()).filter(|s| !s.is_empty()).or_else(|| attr(e, "id")).unwrap_or_default();
    let text = attr(e, "value").filter(|v| !v.is_empty()).or_else(|| wrapper.and_then(|w| w.1.clone())).unwrap_or_default();
    Cell {
        id,
        text,
        html: style.split(';').any(|kv| kv == "html=1"),
        vertex: attr(e, "vertex").as_deref() == Some("1"),
        edge: attr(e, "edge").as_deref() == Some("1"),
        parent: attr(e, "parent"),
        source: attr(e, "source"),
        target: attr(e, "target"),
        ..Default::default()
    }
}

/// Compressed page: base64 → raw deflate → URI-encoded XML (older files
/// skip the URI step, so decode only when the XML isn't already plain).
fn decode_drawio_payload(payload: &str) -> Result<String, String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(payload.split_whitespace().collect::<String>())
        .map_err(|e| format!("draw.io page base64: {e}"))?;
    let text = String::from_utf8_lossy(&inflate(&raw, false)?).into_owned();
    Ok(if text.trim_start().starts_with('<') { text } else { percent_decode(&text) })
}

fn drawio_diagram(pages: &[Page]) -> Diagram {
    let multi = pages.len() > 1;
    let mut d = Diagram::default();
    for (pi, page) in pages.iter().enumerate() {
        let page_frame = multi.then(|| {
            let id = format!("page:{pi}");
            let title = if page.name.trim().is_empty() { format!("Page {}", pi + 1) } else { page.name.clone() };
            d.frames.push(Frame { id: id.clone(), title, parent: None, x: 0.0, y: pi as f64 * PAGE_STRIDE });
            id
        });
        let prefix = if multi { format!("{pi}:") } else { String::new() };
        DrawioPage::new(page, prefix, page_frame).emit(&mut d);
    }
    d
}

struct DrawioPage<'a> {
    by_id: HashMap<&'a str, &'a Cell>,
    cells: &'a [Cell],
    /// Vertices that parent another vertex or edge (containers, groups).
    containers: HashSet<&'a str>,
    /// Edge id → texts of its label cells (vertices parented by the edge).
    edge_labels: HashMap<&'a str, Vec<&'a str>>,
    prefix: String,
    page_frame: Option<String>,
}

impl<'a> DrawioPage<'a> {
    fn new(page: &'a Page, prefix: String, page_frame: Option<String>) -> Self {
        let by_id: HashMap<&str, &Cell> = page.cells.iter().map(|c| (c.id.as_str(), c)).collect();
        let mut containers = HashSet::new();
        let mut edge_labels: HashMap<&str, Vec<&str>> = HashMap::new();
        for c in page.cells.iter().filter(|c| c.vertex || c.edge) {
            let Some(p) = c.parent.as_deref().and_then(|p| by_id.get(p)) else { continue };
            if p.edge && c.vertex {
                edge_labels.entry(p.id.as_str()).or_default().push(c.text.as_str());
            } else if p.vertex {
                containers.insert(p.id.as_str());
            }
        }
        DrawioPage { by_id, cells: &page.cells, containers, edge_labels, prefix, page_frame }
    }

    fn emit(&self, d: &mut Diagram) {
        for c in self.cells {
            let (x, y) = self.abs(c);
            if c.edge {
                d.edges.push(self.edge(c));
            } else if !c.vertex || self.is_edge_label(c) {
                continue;
            } else if self.containers.contains(c.id.as_str()) {
                if !self.text(c).trim().is_empty() {
                    let parent = self.frame_of(c.parent.as_deref());
                    d.frames.push(Frame { id: self.key(&c.id), title: self.text(c), parent, x, y });
                }
            } else {
                let frame = self.frame_of(c.parent.as_deref());
                d.nodes.push(Node { id: self.key(&c.id), text: self.text(c), frame, x, y, h: c.h });
            }
        }
    }

    fn edge(&self, c: &Cell) -> Edge {
        let mut label = vec![self.text(c)];
        for t in self.edge_labels.get(c.id.as_str()).into_iter().flatten() {
            label.push(self.html_text(t, true));
        }
        let end = |id: &Option<String>| id.as_deref().filter(|s| !s.is_empty()).map(|s| self.key(s));
        Edge { from: end(&c.source), to: end(&c.target), label: label.join(" ") }
    }

    fn is_edge_label(&self, c: &Cell) -> bool {
        c.parent.as_deref().and_then(|p| self.by_id.get(p)).is_some_and(|p| p.edge)
    }

    /// Nearest labeled container above `parent`, else the page frame.
    fn frame_of(&self, mut parent: Option<&'a str>) -> Option<String> {
        for _ in 0..=self.cells.len() {
            let Some(p) = parent.and_then(|id| self.by_id.get(id)) else { break };
            if !p.vertex {
                break;
            }
            if self.containers.contains(p.id.as_str()) && !self.text(p).trim().is_empty() {
                return Some(self.key(&p.id));
            }
            parent = p.parent.as_deref();
        }
        self.page_frame.clone()
    }

    /// Child geometry is relative to its container: sum the vertex chain.
    fn abs(&self, c: &Cell) -> (f64, f64) {
        let (mut x, mut y) = (c.x, c.y);
        let mut parent = c.parent.as_deref();
        for _ in 0..self.cells.len() {
            let Some(p) = parent.and_then(|id| self.by_id.get(id)).filter(|p| p.vertex) else { break };
            x += p.x;
            y += p.y;
            parent = p.parent.as_deref();
        }
        (x, y)
    }

    fn text(&self, c: &Cell) -> String {
        self.html_text(&c.text, c.html)
    }

    fn html_text(&self, s: &str, html: bool) -> String {
        if html {
            crate::extract::strip_html(s)
        } else {
            s.to_string()
        }
    }

    fn key(&self, id: &str) -> String {
        format!("{}{id}", self.prefix)
    }
}

// ----- Excalidraw -----

/// Parse an Excalidraw scene (`{"type": "excalidraw", "elements": [...]}`).
pub fn excalidraw_to_text(json: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("excalidraw json: {e}"))?;
    let elements = v["elements"].as_array().ok_or("not an excalidraw scene (no elements)")?;
    Ok(render(&excalidraw_diagram(elements)))
}

fn excalidraw_diagram(elements: &[serde_json::Value]) -> Diagram {
    let live: Vec<&serde_json::Value> = elements.iter().filter(|e| e["isDeleted"].as_bool() != Some(true)).collect();
    let ids: HashSet<&str> = live.iter().filter_map(|e| e["id"].as_str()).collect();
    let s = |e: &serde_json::Value, k: &str| e[k].as_str().unwrap_or("").to_string();
    let container = |e: &serde_json::Value| e["containerId"].as_str().filter(|c| ids.contains(c)).map(str::to_string);
    let mut bound: HashMap<String, Vec<String>> = HashMap::new();
    for e in live.iter().filter(|e| e["type"] == "text") {
        if let Some(c) = container(e) {
            bound.entry(c).or_default().push(excalidraw_text(e));
        }
    }
    let mut d = Diagram::default();
    for e in &live {
        let (id, ty) = (s(e, "id"), s(e, "type"));
        let (x, y, h) = (e["x"].as_f64().unwrap_or(0.0), e["y"].as_f64().unwrap_or(0.0), e["height"].as_f64().unwrap_or(0.0));
        let label = bound.get(&id).map(|t| t.join(" ")).unwrap_or_default();
        let frame = e["frameId"].as_str().map(str::to_string);
        match ty.as_str() {
            "frame" | "magicframe" => d.frames.push(Frame { id, title: s(e, "name"), parent: None, x, y }),
            "arrow" | "line" => {
                let end = |k: &str| e[k]["elementId"].as_str().map(str::to_string);
                d.edges.push(Edge { from: end("startBinding"), to: end("endBinding"), label });
            }
            "text" if container(e).is_some() => {}
            "text" => d.nodes.push(Node { id, text: excalidraw_text(e), frame, x, y, h }),
            "image" | "freedraw" | "embeddable" | "iframe" => {}
            _ => d.nodes.push(Node { id, text: label, frame, x, y, h }),
        }
    }
    d
}

/// `originalText` is the unwrapped text; `text` carries soft line wraps.
fn excalidraw_text(e: &serde_json::Value) -> String {
    e["originalText"].as_str().or_else(|| e["text"].as_str()).unwrap_or("").to_string()
}

/// Embedded scene of an `.excalidraw.svg`: base64 JSON envelope whose
/// `encoded` is a byte string, zlib-compressed when `compressed` is set.
fn excalidraw_svg_scene(svg: &str) -> Option<String> {
    let start = svg.find("<!-- payload-start -->")? + "<!-- payload-start -->".len();
    let end = start + svg[start..].find("<!-- payload-end -->")?;
    let envelope = base64::engine::general_purpose::STANDARD
        .decode(svg[start..end].split_whitespace().collect::<String>())
        .ok()?;
    let env: serde_json::Value = serde_json::from_slice(&envelope).ok()?;
    let encoded = env["encoded"].as_str()?;
    let bytes: Vec<u8> = encoded.chars().map(|c| u8::try_from(c as u32).ok()).collect::<Option<_>>()?;
    let scene = if env["compressed"].as_bool() == Some(true) { inflate(&bytes, true).ok()? } else { bytes };
    String::from_utf8(scene).ok()
}

// ----- SVG -----

/// SVG text: an embedded draw.io/Excalidraw scene wins (full graph with
/// edges), else root `<title>`/`<desc>` + every `<text>` element.
pub fn svg_to_text(svg: &str) -> Result<String, String> {
    if let Some(content) = drawio_svg_content(svg) {
        if let Ok(text) = drawio_to_text(&content) {
            return Ok(text);
        }
    }
    if let Some(scene) = excalidraw_svg_scene(svg) {
        if let Ok(text) = excalidraw_to_text(&scene) {
            return Ok(text);
        }
    }
    svg_text_diagram(svg).map(|d| render(&d))
}

fn drawio_svg_content(svg: &str) -> Option<String> {
    let mut reader = quick_xml::Reader::from_str(svg);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if local_name(&e) == "svg" => {
                return attr(&e, "content").filter(|c| c.contains("<mxfile") || c.contains("<mxGraphModel"));
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

#[derive(Default)]
struct SvgWalk {
    depth: usize,
    /// Root-level `<title>`/`<desc>` being read (depth 2 = child of `<svg>`).
    meta: Option<&'static str>,
    /// Open `<text>` element: (depth, buffer, x, y).
    text: Option<(usize, String, f64, f64)>,
    d: Diagram,
    desc: String,
}

fn svg_text_diagram(svg: &str) -> Result<Diagram, String> {
    let mut reader = quick_xml::Reader::from_str(svg);
    let mut w = SvgWalk::default();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => svg_open(&mut w, &e),
            Ok(Event::End(_)) => svg_close(&mut w),
            Ok(Event::Text(t)) => {
                let raw = t.into_inner().into_owned();
                svg_push(&mut w, &quick_xml::escape::unescape(&raw).map(|c| c.into_owned()).unwrap_or(raw));
            }
            Ok(Event::CData(t)) => svg_push(&mut w, &t.into_inner()),
            Ok(Event::GeneralRef(r)) => {
                if let Some(c) = crate::extract::resolve_ref(&r.into_inner()) {
                    svg_push(&mut w, &c.to_string());
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("svg: {e}")),
            _ => {}
        }
    }
    if !w.desc.trim().is_empty() {
        // The description reads before any drawn text.
        w.d.nodes.insert(0, Node { id: "desc".into(), text: w.desc, x: f64::MIN, y: f64::MIN, ..Default::default() });
    }
    Ok(w.d)
}

fn svg_open(w: &mut SvgWalk, e: &BytesStart) {
    w.depth += 1;
    let name = local_name(e);
    if let Some((_, buf, x, y)) = w.text.as_mut() {
        // `<tspan>` lines: a space between them, and the first one places
        // a `<text>` that has no coordinates of its own.
        buf.push(' ');
        if (*x, *y) == (0.0, 0.0) {
            (*x, *y) = (attr_f64(e, "x"), attr_f64(e, "y"));
        }
        return;
    }
    match name.as_str() {
        "text" => w.text = Some((w.depth, String::new(), attr_f64(e, "x"), attr_f64(e, "y"))),
        "title" if w.depth == 2 && w.d.title.is_empty() => w.meta = Some("title"),
        "desc" if w.depth == 2 && w.desc.is_empty() => w.meta = Some("desc"),
        _ => {}
    }
}

fn svg_close(w: &mut SvgWalk) {
    if let Some((depth, ..)) = &w.text {
        if *depth == w.depth {
            let (_, text, x, y) = w.text.take().expect("checked above");
            let id = format!("t{}", w.d.nodes.len());
            w.d.nodes.push(Node { id, text, x, y, ..Default::default() });
        }
    }
    w.meta = None;
    w.depth = w.depth.saturating_sub(1);
}

fn svg_push(w: &mut SvgWalk, s: &str) {
    if let Some((_, buf, ..)) = w.text.as_mut() {
        buf.push_str(s);
    } else if w.meta == Some("title") {
        w.d.title.push_str(s);
    } else if w.meta == Some("desc") {
        w.desc.push_str(s);
    }
}

// ----- Visio (.vsdx) -----

/// One Visio shape as parsed: page-local cells, its own text, its master,
/// and its group parent (child pins are relative to the group's origin).
#[derive(Debug, Default, Clone)]
struct VShape {
    id: String,
    master: Option<String>,
    text: Option<String>,
    cells: HashMap<String, f64>,
    one_d: bool,
    parent: Option<usize>,
}

impl VShape {
    fn cell(&self, n: &str) -> f64 {
        self.cells.get(n).copied().unwrap_or(0.0)
    }
}

/// `(connector id) → (begin shape, end shape)` from `<Connects>`.
type Connects = HashMap<String, (Option<String>, Option<String>)>;

#[derive(Default)]
struct VsdxWalk {
    shapes: Vec<VShape>,
    /// Open `<Shape>` elements: (index into `shapes`, element depth).
    stack: Vec<(usize, usize)>,
    depth: usize,
    /// Depth of the open `<Text>` of the innermost shape.
    text_depth: Option<usize>,
    connects: Connects,
}

/// Parse a Visio `.vsdx` package: pages in `pages.xml` order (background
/// pages skipped) → frames when there are several, shapes → nodes (own text,
/// else the master's default text), 1-D shapes → edges resolved through
/// `<Connects>` (`BeginX`/`EndX`), connector text → edge label.
pub fn vsdx_to_text(bytes: &[u8]) -> Result<String, String> {
    let mut zip = crate::extract::open_zip(bytes)?;
    let masters = vsdx_master_texts(&mut zip);
    let pages = vsdx_page_index(&mut zip, "visio/pages/pages.xml", "visio/pages/_rels/pages.xml.rels")?;
    let multi = pages.len() > 1;
    let mut d = Diagram::default();
    for (pi, (name, part)) in pages.iter().enumerate() {
        let xml = crate::extract::read_zip_entry(&mut zip, &format!("visio/pages/{part}"))?;
        let walk = vsdx_walk(&xml)?;
        let frame = multi.then(|| {
            let id = format!("page:{pi}");
            let title = if name.trim().is_empty() { format!("Page {}", pi + 1) } else { name.clone() };
            d.frames.push(Frame { id: id.clone(), title, parent: None, x: 0.0, y: pi as f64 * PAGE_STRIDE });
            id
        });
        let prefix = if multi { format!("{pi}:") } else { String::new() };
        vsdx_emit(&walk, &masters, &prefix, frame, pi as f64 * PAGE_STRIDE, &mut d);
    }
    Ok(render(&d))
}

/// Relationship id → target part name, from one `_rels/*.rels` part.
fn vsdx_rels(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, rels: &str) -> HashMap<String, String> {
    let Ok(xml) = crate::extract::read_zip_entry(zip, rels) else { return HashMap::new() };
    let mut out = HashMap::new();
    let mut reader = quick_xml::Reader::from_str(&xml);
    while let Ok(ev) = reader.read_event() {
        match ev {
            Event::Empty(e) | Event::Start(e) if local_name(&e) == "Relationship" => {
                if let (Some(id), Some(target)) = (attr(&e, "Id"), attr(&e, "Target")) {
                    out.insert(id, target.rsplit('/').next().unwrap_or(&target).to_string());
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// `(name, part file)` per foreground page (or master) in index order.
fn vsdx_page_index(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, index: &str, rels: &str) -> Result<Vec<(String, String)>, String> {
    let targets = vsdx_rels(zip, rels);
    let xml = crate::extract::read_zip_entry(zip, index).map_err(|e| format!("not a Visio package: {e}"))?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut out = Vec::new();
    let mut current: Option<(String, bool)> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match local_name(&e).as_str() {
                "Page" | "Master" => {
                    let name = attr(&e, "Name").or_else(|| attr(&e, "NameU")).unwrap_or_default();
                    current = Some((name, attr(&e, "Background").as_deref() == Some("1")));
                }
                "Rel" => {
                    let target = attr(&e, "r:id").and_then(|id| targets.get(&id).cloned());
                    if let (Some((name, false)), Some(t)) = (current.take(), target) {
                        out.push((name, t));
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => return Ok(out),
            Err(e) => return Err(format!("visio index: {e}")),
            _ => {}
        }
    }
}

/// Master id → its default text (first shape text in the master part).
fn vsdx_master_texts(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>) -> HashMap<String, String> {
    let ids = vsdx_master_ids(zip);
    let mut out = HashMap::new();
    for (id, part) in ids {
        let Ok(xml) = crate::extract::read_zip_entry(zip, &format!("visio/masters/{part}")) else { continue };
        let Ok(walk) = vsdx_walk(&xml) else { continue };
        if let Some(t) = walk.shapes.iter().filter_map(|s| s.text.clone()).find(|t| !t.trim().is_empty()) {
            out.insert(id, t);
        }
    }
    out
}

fn vsdx_master_ids(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>) -> Vec<(String, String)> {
    let targets = vsdx_rels(zip, "visio/masters/_rels/masters.xml.rels");
    let Ok(xml) = crate::extract::read_zip_entry(zip, "visio/masters/masters.xml") else { return Vec::new() };
    let mut reader = quick_xml::Reader::from_str(&xml);
    let (mut out, mut current) = (Vec::new(), None);
    while let Ok(ev) = reader.read_event() {
        match ev {
            Event::Start(e) | Event::Empty(e) => match local_name(&e).as_str() {
                "Master" => current = attr(&e, "ID"),
                "Rel" => {
                    if let (Some(id), Some(t)) = (current.take(), attr(&e, "r:id").and_then(|r| targets.get(&r).cloned())) {
                        out.push((id, t));
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

fn vsdx_walk(xml: &str) -> Result<VsdxWalk, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut w = VsdxWalk::default();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                w.depth += 1;
                vsdx_open(&mut w, &e, false);
            }
            Ok(Event::Empty(e)) => vsdx_open(&mut w, &e, true),
            Ok(Event::End(e)) => {
                vsdx_close(&mut w, e.local_name().as_ref());
                w.depth = w.depth.saturating_sub(1);
            }
            Ok(Event::Text(t)) => vsdx_text(&mut w, &quick_xml::escape::unescape(&t.into_inner()).map(|c| c.into_owned()).unwrap_or_default()),
            Ok(Event::GeneralRef(r)) => {
                if let Some(c) = crate::extract::resolve_ref(&r.into_inner()) {
                    vsdx_text(&mut w, &c.to_string());
                }
            }
            Ok(Event::Eof) => return Ok(w),
            Err(e) => return Err(format!("visio page: {e}")),
            _ => {}
        }
    }
}

fn vsdx_open(w: &mut VsdxWalk, e: &BytesStart, empty: bool) {
    // A shape's own cells/text are its direct children (depth + 1);
    // Section rows hold geometry cells that must not override the pin.
    let own = w.stack.last().filter(|(_, d)| w.depth == d + usize::from(!empty)).map(|(i, _)| *i);
    match local_name(e).as_str() {
        "Shape" if !empty => {
            let parent = w.stack.last().map(|(i, _)| *i);
            w.shapes.push(VShape { id: attr(e, "ID").unwrap_or_default(), master: attr(e, "Master"), parent, ..Default::default() });
            w.stack.push((w.shapes.len() - 1, w.depth));
        }
        "Cell" => {
            let (Some(i), Some(n)) = (own, attr(e, "N")) else { return };
            let v = attr(e, "V").and_then(|v| v.parse::<f64>().ok());
            if n == "BeginX" {
                w.shapes[i].one_d = true;
            }
            if let Some(v) = v {
                w.shapes[i].cells.insert(n, v);
            }
        }
        "Text" if !empty && own.is_some() => {
            w.text_depth = Some(w.depth);
            if let Some(i) = own {
                w.shapes[i].text.get_or_insert_with(String::new);
            }
        }
        "Connect" => {
            let (Some(from), Some(cell)) = (attr(e, "FromSheet"), attr(e, "FromCell")) else { return };
            let ends = w.connects.entry(from).or_default();
            match cell.as_str() {
                "BeginX" => ends.0 = attr(e, "ToSheet"),
                "EndX" => ends.1 = attr(e, "ToSheet"),
                _ => {}
            }
        }
        _ => {}
    }
}

fn vsdx_close(w: &mut VsdxWalk, name: &str) {
    match name {
        "Text" if w.text_depth == Some(w.depth) => w.text_depth = None,
        "Shape" if w.stack.last().is_some_and(|(_, d)| *d == w.depth) => {
            w.stack.pop();
        }
        _ => {}
    }
}

fn vsdx_text(w: &mut VsdxWalk, s: &str) {
    if w.text_depth.is_none() {
        return;
    }
    if let Some(text) = w.stack.last().and_then(|(i, _)| w.shapes[*i].text.as_mut()) {
        text.push_str(s);
    }
}

/// Page coordinates: a child's pin is relative to its group's lower-left
/// corner; Visio's y axis points up, the normal form's down.
fn vsdx_abs(shapes: &[VShape], i: usize) -> (f64, f64) {
    let (mut x, mut y) = (shapes[i].cell("PinX"), shapes[i].cell("PinY"));
    let mut parent = shapes[i].parent;
    for _ in 0..shapes.len() {
        let Some(p) = parent else { break };
        let g = &shapes[p];
        let loc = |n: &str, size: &str| g.cells.get(n).copied().unwrap_or(g.cell(size) / 2.0);
        x += g.cell("PinX") - loc("LocPinX", "Width");
        y += g.cell("PinY") - loc("LocPinY", "Height");
        parent = g.parent;
    }
    (x, -y)
}

fn vsdx_emit(w: &VsdxWalk, masters: &HashMap<String, String>, prefix: &str, frame: Option<String>, y_offset: f64, d: &mut Diagram) {
    let key = |id: &str| format!("{prefix}{id}");
    for (i, s) in w.shapes.iter().enumerate() {
        let text = s.text.clone().filter(|t| !t.trim().is_empty()).or_else(|| s.master.as_ref().and_then(|m| masters.get(m).cloned())).unwrap_or_default();
        if s.one_d || w.connects.contains_key(&s.id) {
            let (from, to) = w.connects.get(&s.id).cloned().unwrap_or_default();
            // A connector's master text ("Dynamic connector") is not a label.
            let label = s.text.clone().unwrap_or_default();
            d.edges.push(Edge { from: from.map(|f| key(&f)), to: to.map(|t| key(&t)), label });
            continue;
        }
        let (x, y) = vsdx_abs(&w.shapes, i);
        d.nodes.push(Node { id: key(&s.id), text, frame: frame.clone(), x, y: y + y_offset, h: s.cell("Height") });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDER_FLOW: &str = r#"<mxfile host="app.diagrams.net"><diagram id="p1" name="Order flow"><mxGraphModel><root>
<mxCell id="0"/><mxCell id="1" parent="0"/>
<mxCell id="lane" value="Payments" style="swimlane;html=1;" vertex="1" parent="1"><mxGeometry x="0" y="0" width="600" height="200" as="geometry"/></mxCell>
<mxCell id="auth" value="Payment &lt;b&gt;authorized&lt;/b&gt;" style="rounded=1;html=1;" vertex="1" parent="lane"><mxGeometry x="20" y="40" width="120" height="60" as="geometry"/></mxCell>
<mxCell id="cap" value="Capture funds" style="rounded=1;" vertex="1" parent="lane"><mxGeometry x="220" y="40" width="120" height="60" as="geometry"/></mxCell>
<UserObject label="Ship order" id="ship"><mxCell style="rounded=1;" vertex="1" parent="1"><mxGeometry x="20" y="400" width="120" height="60" as="geometry"/></mxCell></UserObject>
<mxCell id="grp" value="" style="group" vertex="1" parent="1"><mxGeometry x="400" y="400" width="200" height="100" as="geometry"/></mxCell>
<mxCell id="note" value="Ops&#10;on call" vertex="1" parent="grp"><mxGeometry x="0" y="0" width="80" height="40" as="geometry"/></mxCell>
<mxCell id="e1" value="" style="edgeStyle=orthogonal;" edge="1" parent="1" source="auth" target="cap"><mxGeometry relative="1" as="geometry"/></mxCell>
<mxCell id="e1l" value="same day" style="edgeLabel;html=1;" vertex="1" connectable="0" parent="e1"><mxGeometry x="-0.1" relative="1" as="geometry"/></mxCell>
<mxCell id="e2" value="then" edge="1" parent="1" source="cap" target="ship"><mxGeometry relative="1" as="geometry"/></mxCell>
<mxCell id="e3" value="escalate" edge="1" parent="1" source="cap"><mxGeometry relative="1" as="geometry"/></mxCell>
</root></mxGraphModel></diagram></mxfile>"#;

    /// draw.io's own encoding of a two-node page (generated with the
    /// encodeURIComponent → deflateRaw → base64 pipeline the editor uses).
    const COMPRESSED_PAGE: &str = "xZPRTsQgEEW/hnekxh+o7j754h+Q5VpIKNRh2m3/XrYQ16au0cTEBxLmDnNnDgHRtP18JD3Y52jgRfMkmpZi5LLr5xbeCyWdEc2jUErmJdThRvZuzcpBEwL/pCCVgkn7EUV5wesYTNYIbyMSw9QjIMZ8s80q1R5HxB5MSz5SC+7LFHLZhmdn2Nb6Oqi0cJ2tpg9V06nE3YfxFSlvKtXXhLwjPLigwwkX32GgOCH9AeF/ImKHeHZsXcjaRTd6qYQwHb7nUzLFkU7YvA7W1IE/Xef+Fghes5u27r9gyuH10a+5zZ94Bw==";

    #[test]
    fn drawio_containers_labels_groups_and_edges() {
        let text = drawio_to_text(ORDER_FLOW).expect("parse");
        assert_eq!(
            text,
            "- Ship order\n- Ops on call\n\n## Payments\n\n- Payment authorized\n- Capture funds\n\n- Payment authorized → Capture funds: same day\n- Capture funds → Ship order: then\n- Capture funds → ?: escalate\n"
        );
    }

    #[test]
    fn drawio_compressed_and_multi_page() {
        let xml = format!(
            r#"<mxfile><diagram name="Refunds">{COMPRESSED_PAGE}</diagram><diagram name="">
<mxGraphModel><root><mxCell id="0"/><mxCell id="1" parent="0"/><mxCell id="a" value="Close ticket" vertex="1" parent="1"><mxGeometry x="0" y="0" width="10" height="10" as="geometry"/></mxCell></root></mxGraphModel></diagram></mxfile>"#
        );
        assert_eq!(
            drawio_to_text(&xml).expect("parse"),
            "## Refunds\n\n- Refund requested\n- Finance approves\n\n- Refund requested → Finance approves: within 2 days\n\n## Page 2\n\n- Close ticket\n"
        );
    }

    #[test]
    fn drawio_bare_model_and_rejects() {
        let bare = r#"<mxGraphModel><root><mxCell id="0"/><mxCell id="1" parent="0"/><mxCell id="a" value="Solo" vertex="1" parent="1"/></root></mxGraphModel>"#;
        assert_eq!(drawio_to_text(bare).expect("parse"), "- Solo\n");
        assert!(drawio_to_text("<html><body>nope</body></html>").is_err());
        assert!(drawio_to_text("<mxfile><diagram>%%%not-base64</diagram></mxfile>").is_err());
        assert!(drawio_to_text("<mxfile><diagram><unclosed").is_err());
    }

    #[test]
    fn excalidraw_frames_bound_text_and_arrows() {
        let scene = r#"{"type":"excalidraw","elements":[
            {"id":"f","type":"frame","name":"Hiring","x":0,"y":0},
            {"id":"a","type":"rectangle","x":10,"y":10,"height":50,"frameId":"f"},
            {"id":"ta","type":"text","text":"Phone\nscreen","originalText":"Phone screen","containerId":"a","x":12,"y":12},
            {"id":"b","type":"diamond","x":300,"y":10,"height":50,"frameId":"f"},
            {"id":"tb","type":"text","text":"Onsite","containerId":"b","x":302,"y":12},
            {"id":"arr","type":"arrow","startBinding":{"elementId":"a"},"endBinding":{"elementId":"b"},"x":0,"y":0},
            {"id":"tl","type":"text","text":"passed","containerId":"arr","x":0,"y":0},
            {"id":"free","type":"text","text":"Owner: recruiting","x":0,"y":-100},
            {"id":"gone","type":"text","text":"deleted note","isDeleted":true,"x":0,"y":0},
            {"id":"orphan","type":"text","text":"Orphaned label","containerId":"gone-box","x":500,"y":-100},
            {"id":"pic","type":"image","x":0,"y":0},
            {"id":"stroke","type":"line","x":0,"y":0}
        ]}"#;
        assert_eq!(
            excalidraw_to_text(scene).expect("parse"),
            "- Owner: recruiting\n- Orphaned label\n\n## Hiring\n\n- Phone screen\n- Onsite\n\n- Phone screen → Onsite: passed\n"
        );
        assert!(excalidraw_to_text(r#"{"type":"other"}"#).is_err());
        assert!(excalidraw_to_text("not json").is_err());
    }

    #[test]
    fn svg_text_title_desc_and_tspans() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><title>Network map</title><desc>Office LAN &amp; VPN</desc>
<g><title>tooltip, not the title</title><text x="200" y="10">Router</text>
<text><tspan x="10" y="10">Core</tspan><tspan x="10" y="24">switch</tspan></text>
<text x="10 20 30" y="300">VPN <![CDATA[gateway]]></text></g></svg>"#;
        assert_eq!(svg_to_text(svg).expect("parse"), "# Network map\n\n- Office LAN & VPN\n- Core switch\n- Router\n- VPN gateway\n");
        assert_eq!(svg_to_text(r#"<svg><path d="M0 0"/></svg>"#).expect("icon"), "");
        assert!(svg_to_text("<svg><text>broken</svg>").is_err());
    }

    #[test]
    fn svg_with_embedded_drawio_or_excalidraw_scene() {
        let escaped = quick_xml::escape::escape(ORDER_FLOW).into_owned();
        let drawio_svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" content="{escaped}"><text>Text is not SVG - cannot display</text></svg>"#);
        assert!(svg_to_text(&drawio_svg).expect("parse").contains("Payment authorized → Capture funds: same day"));

        let payload = "eyJ2ZXJzaW9uIjogIjEiLCAiZW5jb2RpbmciOiAiYnN0cmluZyIsICJjb21wcmVzc2VkIjogdHJ1ZSwgImVuY29kZWQiOiAieFx1MDA5Y1x1MDBhNVx1MDA5Mk1cdTAwOGJcdTAwYzIwXHUwMDEwXHUwMDg2XHUwMGZmSlx1MDBjOVx1MDBkOUNbXHUwMDExXHUwMGM0XHUwMGEzXHUwMGFjXHUwMDg3XHUwMGJkeVx1MDAxN1x1MDAwZkkzXHUwMGI2XHUwMDAzXHUwMGRkXHUwMGE5XHUwMGE0XHUwMGEzXHUwMGFkXHUwMDE0XHUwMGZmXHUwMGJiXHUwMDEzaTBlXVx1MDAxN1x1MDBmMVx1MDA5NFx1MDBiY1x1MDBmM1x1MDBmNVx1MDBlNFx1MDAxZDIoXHUwMGJlXHUwMDFjQVx1MDBhZFx1MDAxMlx1MDAwNX1cdTAwYTFrXHUwMGI0TndqXHUwMDk2XHUwMGE4M1x1MDBiOFx1MDAxNlx1MDAxYlx1MDA5MkQuXG5qXHUwMGY4XHUwMDAxXHUwMGUyVlx1MDBlNG5QaH1cdTAwYmRcdTAwZjZlXHUwMGExXHUwMGQ5QVx1MDBjMVx1MDA5YVx1MDBjYVx1MDAxYXxcdTAwYjBcdTAwOTdIKlx1MDBlN2U8O1x1MDBiNFxcXHUwMGM5PUtcdTAwYmRcdTAwYWFcdTAwMDBcdTAwY2JcdTAwOGFFLlx1MDBiYzJcdTAwY2RcdTAwODlcdTAwZWNcdTAwZTZcdTAwYzl4XHUwMDllXHUwMGNjZ1x1MDBlOFldXHUwMGY3XHUwMGQ3WVx1MDBmMlwiP1x1MDBhMlx1MDBiM1x1MDBjMFx1MDBjZWJcdTAwZjhyXHUwMGMyXHUwMGNlXHUwMGJkXHUwMGJhN0lcdTAwZjNcdTAwOTdcdTAwZDNcdTAwODd7d1x1MDBkMVx1MDAxMGskcFx1MDBkZlx1MDBhM1x1MDBjM1x1MDA4OGdeXHUwMGRhXHUwMDlkXHUwMGE3XHUwMDlmXHUwMDFiNlx1MDBmZlx1MDAxOFx1MDBmZVx1MDA5ZFx1MDAwZlx1MDBmMFx1MDBiN1x1MDAxZG9PXHUwMGE2XHUwMGM2XHUwMGI2elx1MDBlMlx1MDBkOVx1MDBjNFx1MDA5ZVx1MDBiNXMxU2RcdTAwZDM9XHUwMGI2XHUwMDFjXHUwMGEwXHUwMGY5XCJcdTAwODJcdTAwZTZTXHUwMGNiXlx1MDBiNFx1MDBhY1x1MDAxZFx1MDBhZlx1MDA5MSxSKVx1MDBhMSFcdTAwZmNcdTAwYTZoXHUwMGM5XG5cdTAwYzhcdTAwZmVZIU9cdTAwOTJFXHUwMGRjXHUwMDAwbVx1MDBiOFx1MDBjMDcifQ==";
        let ex_svg = format!("<svg><!-- svg-source:excalidraw --><!-- payload-type:application/vnd.excalidraw+json --><!-- payload-version:2 --><!-- payload-start -->{payload}<!-- payload-end --><text x=\"0\" y=\"0\">Draft</text></svg>");
        assert_eq!(svg_to_text(&ex_svg).expect("parse"), "- Draft\n- Publish\n\n- Draft → Publish\n");

        // A corrupt payload falls back to the drawn text, never fails the file.
        let broken = "<svg><!-- payload-start -->@@@<!-- payload-end --><text>Draft</text></svg>";
        assert_eq!(svg_to_text(broken).expect("fallback"), "- Draft\n");
    }

    fn vsdx(parts: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in parts {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    const NS: &str = "xmlns='http://schemas.microsoft.com/office/visio/2012/main' xmlns:r='http://schemas.openxmlformats.org/officeDocument/2006/relationships'";

    fn rels(targets: &[(&str, &str)]) -> String {
        let rows: String = targets.iter().map(|(id, t)| format!("<Relationship Id='{id}' Type='x' Target='{t}'/>")).collect();
        format!("<Relationships xmlns='http://schemas.openxmlformats.org/package/2006/relationships'>{rows}</Relationships>")
    }

    #[test]
    fn vsdx_pages_masters_groups_and_connects() {
        let pages = format!("<Pages {NS}><Page ID='0' Name='Flow'><PageSheet/><Rel r:id='rId1'/></Page><Page ID='1' Name='Bg' Background='1'><Rel r:id='rId2'/></Page><Page ID='2' NameU='Notes'><Rel r:id='rId3'/></Page></Pages>");
        let page1 = format!("<PageContents {NS}><Shapes>\
            <Shape ID='1' Type='Shape' Master='2'><Cell N='PinX' V='1'/><Cell N='PinY' V='9'/><Cell N='Height' V='0.5'/><Text>Order &amp; received</Text></Shape>\
            <Shape ID='2' Type='Shape' Master='3'><Cell N='PinX' V='4'/><Cell N='PinY' V='9'/><Cell N='Height' V='0.5'/></Shape>\
            <Shape ID='3' Type='Shape'><Cell N='BeginX' V='1.5'/><Cell N='PinX' V='2.5'/><Cell N='PinY' V='9'/><Text>check stock</Text></Shape>\
            <Shape ID='4' Type='Group'><Cell N='PinX' V='2'/><Cell N='PinY' V='5'/><Cell N='Width' V='4'/><Cell N='Height' V='2'/>\
              <Section N='Geometry'><Row T='MoveTo'><Cell N='PinX' V='77'/><Cell N='PinY' V='77'/></Row></Section>\
              <Shapes><Shape ID='5'><Cell N='PinX' V='1'/><Cell N='PinY' V='1'/><Cell N='Height' V='0.5'/><Text>Pack<cp IX='0'/>ing</Text></Shape></Shapes>\
            </Shape></Shapes>\
            <Connects><Connect FromSheet='3' FromCell='BeginX' ToSheet='1' ToCell='PinX'/><Connect FromSheet='3' FromCell='EndX' ToSheet='2' ToCell='PinX'/></Connects></PageContents>");
        let page3 = format!("<PageContents {NS}><Shapes>\
            <Shape ID='1'><Cell N='PinX' V='1'/><Cell N='PinY' V='2'/><Text>Ship it</Text></Shape>\
            <Shape ID='2'><Cell N='BeginX' V='0'/><Text>loose</Text></Shape>\
            </Shapes><Connects><Connect FromSheet='2' FromCell='BeginX' ToSheet='1'/></Connects></PageContents>");
        let bg = format!("<PageContents {NS}><Shapes><Shape ID='1'><Text>Confidential watermark</Text></Shape></Shapes></PageContents>");
        let masters = format!("<Masters {NS}><Master ID='3' NameU='Decision'><Rel r:id='rId1'/></Master></Masters>");
        let master1 = format!("<MasterContents {NS}><Shapes><Shape ID='5'><Text>Decision</Text></Shape></Shapes></MasterContents>");
        let bytes = vsdx(&[
            ("visio/pages/pages.xml", &pages),
            ("visio/pages/_rels/pages.xml.rels", &rels(&[("rId1", "page1.xml"), ("rId2", "page2.xml"), ("rId3", "page3.xml")])),
            ("visio/pages/page1.xml", &page1),
            ("visio/pages/page2.xml", &bg),
            ("visio/pages/page3.xml", &page3),
            ("visio/masters/masters.xml", &masters),
            ("visio/masters/_rels/masters.xml.rels", &rels(&[("rId1", "master1.xml")])),
            ("visio/masters/master1.xml", &master1),
        ]);
        assert_eq!(
            vsdx_to_text(&bytes).expect("parse"),
            "## Flow\n\n- Order & received\n- Decision\n- Packing\n\n- Order & received → Decision: check stock\n\n## Notes\n\n- Ship it\n\n- Ship it → ?: loose\n"
        );
        assert_eq!(crate::extract::extract_bytes("vsdx", &bytes).unwrap(), vsdx_to_text(&bytes).unwrap());
        assert!(vsdx_to_text(b"not a zip").is_err());
        assert!(vsdx_to_text(&vsdx(&[("word/document.xml", "<w/>")])).is_err(), "no pages.xml → not Visio");
    }

    #[test]
    fn percent_decode_handles_utf8_and_malformed_escapes() {
        assert_eq!(percent_decode("%3Ca%20b%3E%C3%A9"), "<a b>é");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
    }
}
