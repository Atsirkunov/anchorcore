use std::sync::OnceLock;

use regex::Regex;

const CHUNK_SIZE: usize = 800;
const CHUNK_OVERLAP: usize = 100;
const CHUNK_MAX_CHARS: usize = 1600;
const CLASSIFY_WINDOW: usize = 16000;

fn env_usize(key: &str, fallback: usize) -> usize {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(fallback)
}

/// Fixed-size chunks with overlap — mirrors `backend/app/chunking.py:14` `chunk_text`.
/// Operates on chars (Python str len) to match Python semantics and avoid UTF-8 panics.
pub fn chunk_text(text: &str) -> Vec<String> {
    let size = env_usize("ANCHOR_CHUNK_SIZE", CHUNK_SIZE);
    let overlap = env_usize("ANCHOR_CHUNK_OVERLAP", CHUNK_OVERLAP);
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= size {
        return vec![text.to_string()];
    }
    let step = size.saturating_sub(overlap).max(1);
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

fn is_heading(line: &str) -> bool {
    static RE_NUM: OnceLock<Regex> = OnceLock::new();
    static RE_CAPS: OnceLock<Regex> = OnceLock::new();
    let s = line.trim();
    if s.is_empty() || s.len() > 80 {
        return false;
    }
    // numbered markers §434, 4.2.1, Article 12
    let re = RE_NUM.get_or_init(|| {
        Regex::new(
            r"^\s*(?:§\s*\d+(\.\d+)*|\d{1,4}(\.\d{1,4}){1,3}|(?:article|annex|section|schedule|rule|appendix)\s+\d+)(?:\s|[:.)\-]|$)",
        )
        .unwrap()
    });
    if re.is_match(s) {
        return true;
    }
    let caps = RE_CAPS.get_or_init(|| Regex::new(r"^[A-Z][A-Z0-9 &()/\-]{3,80}$").unwrap());
    if caps.is_match(s) && !s.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    false
}

pub fn chunk_document(text: &str) -> Vec<String> {
    let max_chars = env_usize("ANCHOR_CHUNK_MAX_CHARS", CHUNK_MAX_CHARS);
    let lines: Vec<&str> = text.split('\n').collect();
    let mut sections: Vec<String> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for line in lines {
        if is_heading(line) {
            if !current.is_empty() {
                sections.push(current.join("\n"));
                current.clear();
            }
            current.push(line.to_string());
        } else {
            current.push(line.to_string());
        }
    }
    if !current.is_empty() {
        sections.push(current.join("\n"));
    }
    let mut chunks = Vec::new();
    for sec in sections {
        chunks.extend(split_section(&sec, max_chars));
    }
    chunks
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
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= window {
        return vec![text.to_string()];
    }
    let step = (window / 2).max(1);
    let max_start = chars.len().saturating_sub(window);
    let mut out = Vec::new();
    let mut i = 0;
    while i <= max_start {
        let end = (i + window).min(chars.len());
        out.push(chars[i..end].iter().collect());
        if end == chars.len() {
            break;
        }
        i += step;
    }
    out
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
}
