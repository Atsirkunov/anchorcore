//! IDF signal score for the embedding gate (`Embedder::embed_gated`).
//! R45.18: was `distill.rs` — distillation is deleted, the score stays.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

fn token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[\p{L}\p{N}_]{3,}").unwrap())
}

fn tokens(text: &str) -> Vec<String> {
    token_re().find_iter(&text.to_lowercase()).map(|m| m.as_str().to_string()).collect()
}

/// R30.7: corpus frequency map built ONCE per doc, scored per chunk.
/// Calling `signal()` per chunk rebuilds this map every time — O(chunks ×
/// corpus) — and an 8401-chunk file never finished (probe: 100→1.0 s,
/// 400→14.5 s, 1600→221 s). `embed_gated` builds once; the gate is O(corpus).
pub struct DocFreq {
    df: HashMap<String, usize>,
    n: f64,
}

impl DocFreq {
    pub fn build(corpus: &[String]) -> Self {
        let mut df = HashMap::new();
        for c in corpus {
            for t in tokens(c) {
                *df.entry(t).or_insert(0) += 1;
            }
        }
        Self { df, n: corpus.len().max(1) as f64 }
    }

    pub fn signal(&self, content: &str) -> f64 {
        score_against(&self.df, self.n, content)
    }
}

fn score_against(doc_freq: &HashMap<String, usize>, n: f64, content: &str) -> f64 {
    let toks = tokens(content);
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

/// B18 IDF-gated signal score (archived `distill.py` `signal` parity).
///
/// `content` is one chunk's text; `corpus` is all pending chunks' texts for the item.
/// Tokens are `[a-z0-9_]{3,}` lowercased. Mean IDF = avg log((n+1)/(df+1)).
///
/// Thin wrapper over `DocFreq` — kept for single-shot callers. Scoring a
/// whole doc must build the map once, not call this per chunk (R30.7).
pub fn signal(content: &str, corpus: &[String]) -> f64 {
    DocFreq::build(corpus).signal(content)
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
    #[test]
    fn doc_freq_matches_repeated_signal() {
        // R30.7: the hoisted map must score identically to per-call signal.
        // Epsilon, not exact: both sum over a HashSet whose iteration order
        // varies per instance, so the old code was already ulp-nondeterministic.
        let corpus: Vec<String> = (0..50)
            .map(|i| format!("Section {i} heading. The migration proceeds in three phases with rollback checkpoints."))
            .collect();
        let freq = super::DocFreq::build(&corpus);
        for c in corpus.iter().chain(std::iter::once(&"unseen rare zebra".to_string())) {
            let (a, b) = (freq.signal(c), super::signal(c, &corpus));
            assert!((a - b).abs() < 1e-9, "hoisted {a} vs per-call {b} for {c:?}");
        }
    }
}
