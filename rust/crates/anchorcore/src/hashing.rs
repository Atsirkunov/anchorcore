use sha2::{Digest, Sha256};

/// sha256 hex of UTF-8 bytes — mirrors `backend/app/hashing.py:13` `window_hash`.
pub fn window_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

/// Alias for content_hash — mirrors `backend/app/hashing.py:17`.
pub fn content_hash(text: &str) -> String {
    window_hash(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash() {
        assert_eq!(
            window_hash("hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
    #[test]
    fn content_hash_alias() {
        assert_eq!(content_hash("hello"), window_hash("hello"));
    }
}
