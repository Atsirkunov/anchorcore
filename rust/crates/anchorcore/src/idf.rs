//! IDF signal score for the embedding gate (`Embedder::embed_gated`).
//! R45.18: was `distill.rs` — distillation is deleted, the score stays.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

/// B18 IDF-gated signal score (archived `distill.py` `signal` parity).
///
/// `content` is one chunk's text; `corpus` is all pending chunks' texts for the item.
/// Tokens are `[a-z0-9_]{3,}` lowercased. Mean IDF = avg log((n+1)/(df+1)).
pub fn signal(content: &str, corpus: &[String]) -> f64 {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"[\p{L}\p{N}_]{3,}").unwrap());
    let mut doc_freq: HashMap<String, usize> = HashMap::new();
    for c in corpus {
        for m in re.find_iter(&c.to_lowercase()) {
            *doc_freq.entry(m.as_str().to_string()).or_insert(0) += 1;
        }
    }
    let n = corpus.len().max(1) as f64;
    let toks: Vec<String> = re.find_iter(&content.to_lowercase()).map(|m| m.as_str().to_string()).collect();
    if toks.is_empty() {
        return 0.0;
    }
    let uniq: std::collections::HashSet<String> = toks.into_iter().collect();
    let mut total = 0.0;
    for t in &uniq {
        if let Some(df) = doc_freq.get(t) {
            if *df == 0 {
                continue;
            }
            let idf = ((n + 1.0) / (*df as f64 + 1.0)).ln();
            total += idf.max(0.0);
        }
    }
    total / (uniq.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signal_low_for_filler() {
        let corpus = vec!["hello world hello".to_string(), "hello world test".to_string()];
        let s = signal("hello world", &corpus);
        assert!(s < 0.5);
    }
    #[test]
    fn signal_high_for_rare() {
        let corpus = vec!["alpha beta gamma".to_string(), "alpha beta delta".to_string(), "alpha beta epsilon".to_string()];
        let s_common = signal("alpha beta", &corpus);
        let s_rare = signal("gamma unique", &corpus);
        assert!(s_rare > s_common);
    }
}
