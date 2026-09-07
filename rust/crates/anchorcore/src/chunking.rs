use std::sync::OnceLock;

use regex::Regex;

const CHUNK_SIZE: usize = 800;
const CHUNK_OVERLAP: usize = 100;
const CHUNK_MAX_CHARS: usize = 1600;
const CLASSIFY_WINDOW: usize = 16000;

fn env_usize(key: &str, fallback: usize) -> usize {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(fallback)
}

/// Shared sliding window over chars (Python str-len semantics, UTF-8 safe).
/// Powers `chunk_text` and `classify_windows` — same loop, one place.
fn slide_windows(text: &str, size: usize, step: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= size {
        return vec![text.to_string()];
    }
    let step = step.max(1);
    let max_start = chars.len().saturating_sub(size);
    let mut out = Vec::new();
    let mut i = 0;
    while i <= max_start {
        let end = (i + size).min(chars.len());
        out.push(chars[i..end].iter().collect());
        if end == chars.len() {
            break;
        }
        i += step;
    }
    // handle trailing remainder when max_start not reached due to step misalignment
    // Python: range(0, max(len-size+1,1), step) ensures last window may be partial
    // Our loop already covers that because we iterate until <= max_start.
    // Edge: if chars.len() % step != 0 the last chunk is already included as above.
    // Ensure we don't miss a final partial chunk when step >1 and remainder < size
    // Example len=801 size=800 step=700 -> max_start=1 -> i=0,700 => two chunks, second 700..801 partial 101 chars, covered.
    out
}

/// Fixed-size chunks with overlap — mirrors `backend/app/chunking.py:14` `chunk_text`.
/// Operates on chars (Python str len) to match Python semantics and avoid UTF-8 panics.
pub fn chunk_text(text: &str) -> Vec<String> {
    let size = env_usize("ANCHOR_CHUNK_SIZE", CHUNK_SIZE);
    let overlap = env_usize("ANCHOR_CHUNK_OVERLAP", CHUNK_OVERLAP);
    slide_windows(text, size, size.saturating_sub(overlap).max(1))
}

/// Single heading classifier — was `is_heading` + `heading_level` (+ 3 level
/// helpers), which evaluated the same regexes twice. Returns the section level
/// (`Some`) or `None` for body text. Level rules are unchanged: §-forms carry
/// dots (`§434`→1, `4.2.1`-style→1+dots), article/annex/section words and
/// ALL-CAPS lines are level 1.
fn classify_heading(line: &str) -> Option<i32> {
    static RE_NUM: OnceLock<Regex> = OnceLock::new();
    static RE_CAPS: OnceLock<Regex> = OnceLock::new();
    let s = line.trim();
    if s.is_empty() || s.len() > 80 {
        return None;
    }
    // numbered markers §434, 4.2.1, Article 12
    let re = RE_NUM.get_or_init(|| {
        Regex::new(
            r"^\s*(?:§\s*\d+(\.\d+)*|\d{1,4}(\.\d{1,4}){1,3}|(?:article|annex|section|schedule|rule|appendix)\s+\d+)(?:\s|[:.)\-]|$)",
        )
        .unwrap()
    });
    let numbered = re.is_match(s);
    let caps = RE_CAPS.get_or_init(|| Regex::new(r"^[A-Z][A-Z0-9 &()/\-]{3,80}$").unwrap());
    if !numbered && !(caps.is_match(s) && !s.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    Some(numbered_heading_level(s))
}

/// Level for a line already known to be a heading (see `classify_heading`).
fn numbered_heading_level(s: &str) -> i32 {
    if s.starts_with('§') {
        let after = s.trim_start_matches('§').trim_start();
        let prefix: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        return 1 + prefix.matches('.').count() as i32;
    }
    let lower = s.to_lowercase();
    if lower.starts_with("article ")
        || lower.starts_with("annex ")
        || lower.starts_with("section ")
        || lower.starts_with("schedule ")
        || lower.starts_with("rule ")
        || lower.starts_with("appendix ")
    {
        return 1;
    }
    if let Some(c) = s.chars().next() {
        if c.is_ascii_digit() {
            let token = s.split(&[' ', ':', ')', '-', '\t'][..]).next().unwrap_or("");
            let token = token.trim_matches(|c| c == '.' || c == ')' || c == ':');
            if token.contains('.') {
                return 1 + token.matches('.').count() as i32;
            }
        }
    }
    1
}

pub fn chunk_document(text: &str) -> Vec<String> {
    chunk_document_with_sections(text).1.into_iter().map(|c| c.content).collect()
}

/// Hierarchical TOC structures for R14.1.
#[derive(Debug, Clone)]
pub struct SectionMeta {
    pub title: String,
    pub level: i32,
    pub parent: Option<usize>,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct ChunkMeta {
    pub content: String,
    pub section_idx: Option<usize>,
    pub level: i32,
    pub path: String,
}

pub fn chunk_document_with_sections(text: &str) -> (Vec<SectionMeta>, Vec<ChunkMeta>) {
    let max_chars = env_usize("ANCHOR_CHUNK_MAX_CHARS", CHUNK_MAX_CHARS);
    let lines: Vec<&str> = text.split('\n').collect();
    // intermediate per-section lines with meta
    struct RawSection {
        title: String,
        level: i32,
        parent: Option<usize>,
        path: String,
        lines: Vec<String>,
    }
    let mut raw_sections: Vec<RawSection> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    for line in lines {
        if let Some(level) = classify_heading(line) {
            let title = line.trim().to_string();
            // pop stack to find parent
            while let Some(&top) = stack.last() {
                if raw_sections[top].level >= level {
                    stack.pop();
                } else {
                    break;
                }
            }
            let parent = stack.last().copied();
            let path = if let Some(p) = parent {
                format!("{} > {}", raw_sections[p].path, title)
            } else {
                title.clone()
            };
            let idx = raw_sections.len();
            raw_sections.push(RawSection {
                title,
                level,
                parent,
                path,
                lines: vec![line.to_string()],
            });
            stack.push(idx);
        } else {
            if raw_sections.is_empty() {
                // root section for leading content before first heading
                let idx = raw_sections.len();
                raw_sections.push(RawSection {
                    title: String::new(),
                    level: 0,
                    parent: None,
                    path: String::new(),
                    lines: Vec::new(),
                });
                stack.push(idx);
            }
            if let Some(&top) = stack.last() {
                raw_sections[top].lines.push(line.to_string());
            }
        }
    }

    // if no sections (empty text), create empty root
    if raw_sections.is_empty() {
        raw_sections.push(RawSection {
            title: String::new(),
            level: 0,
            parent: None,
            path: String::new(),
            lines: Vec::new(),
        });
    }

    // build SectionMeta vec
    let metas: Vec<SectionMeta> = raw_sections
        .iter()
        .map(|r| SectionMeta {
            title: r.title.clone(),
            level: r.level,
            parent: r.parent,
            path: r.path.clone(),
        })
        .collect();

    // build chunks per section
    let mut chunks: Vec<ChunkMeta> = Vec::new();
    for (idx, rs) in raw_sections.iter().enumerate() {
        let text = rs.lines.join("\n");
        if text.trim().is_empty() {
            continue;
        }
        let parts = split_section(&text, max_chars);
        for p in parts {
            chunks.push(ChunkMeta {
                content: p,
                section_idx: Some(idx),
                level: rs.level,
                path: rs.path.clone(),
            });
        }
    }

    // if raw_sections was root-only with empty title, keep section but chunks will map to it
    // filter empty metas? keep all for tree completeness (root with level 0 may be omitted if empty title and no parent)
    // Keep metas as is; pipeline will skip inserting empty-title root if it has no content? but we keep it for now.
    (metas, chunks)
}

fn split_section(section: &str, max_chars: usize) -> Vec<String> {
    static RE_SPLIT: OnceLock<Regex> = OnceLock::new();
    if section.chars().count() <= max_chars {
        return vec![section.to_string()];
    }
    let re = RE_SPLIT.get_or_init(|| Regex::new(r"\n\s*\n").unwrap());
    let paras: Vec<String> = re
        .split(section)
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for para in paras {
        if para.chars().count() > max_chars {
            if !cur.is_empty() {
                out.push(cur.clone());
                cur.clear();
            }
            out.extend(chunk_text(&para));
        } else if cur.chars().count() + para.chars().count() + 2 <= max_chars {
            if cur.is_empty() {
                cur = para;
            } else {
                cur = format!("{}\n\n{}", cur, para);
            }
        } else {
            if !cur.is_empty() {
                out.push(cur.clone());
            }
            cur = para;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn classify_windows(text: &str) -> Vec<String> {
    let window = env_usize("ANCHOR_CLASSIFY_WINDOW_CHARS", CLASSIFY_WINDOW);
    slide_windows(text, window, (window / 2).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunk_text_basic() {
        let t = "a".repeat(2000);
        let c = chunk_text(&t);
        assert!(c.len() >= 2);
        // first chunk size 800
        assert_eq!(c[0].len(), 800);
    }
    #[test]
    fn chunk_doc_heading() {
        let doc = "§434 Merger\ncontent\n\n§437 Dates\njan";
        let c = chunk_document(doc);
        assert_eq!(c.len(), 2);
    }
    #[test]
    fn classify_window() {
        let t = "a".repeat(40000);
        let w = classify_windows(&t);
        assert!(w.len() >= 3);
    }
    #[test]
    fn chunk_text_unicode_safe() {
        let t = "é".repeat(2000); // 2 bytes per char, 2000 chars
        let c = chunk_text(&t);
        assert!(c.len() >= 2);
        assert!(c.iter().all(|s| s.chars().count() <= 800));
    }
    #[test]
    fn chunk_with_sections_hierarchy() {
        let doc = "§434 Merger\ncontent a\n\n§437 Dates\njan\n\n4.2.1 Sub\nsub content";
        let (secs, chunks) = chunk_document_with_sections(doc);
        // 3 sections: §434, §437, 4.2.1 (child of §437)
        assert_eq!(secs.len(), 3);
        assert_eq!(secs[0].level, 1);
        assert_eq!(secs[1].level, 1);
        assert_eq!(secs[2].level, 3);
        assert_eq!(secs[2].parent, Some(1));
        assert_eq!(secs[2].path, "§437 Dates > 4.2.1 Sub");
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[2].section_idx, Some(2));
        assert_eq!(chunks[2].path, "§437 Dates > 4.2.1 Sub");
    }
    #[test]
    fn chunk_with_sections_root() {
        let doc = "plain intro\nno heading\n\n§434 Merger\ncontent";
        let (secs, chunks) = chunk_document_with_sections(doc);
        assert!(secs.iter().any(|s| s.level == 0));
        assert_eq!(chunks.len(), 2);
    }
}
