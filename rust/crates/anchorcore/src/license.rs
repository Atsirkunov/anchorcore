//! R19.1 (B59): Team edition license check — offline, no phone-home.
//!
//! Rust port of `backend/app/license.py` (frozen on tag `archive/python-final`),
//! format-compatible with the existing issuer `scripts/make_license.py`.
//!
//! The personal local app never touches this module (`ANCHOR_EDITION` unset).
//! The Docker/Team image bakes `ANCHOR_EDITION=team` and calls
//! [`enforce_team_license`] at startup: a missing or forged license refuses to
//! boot (exit ≠ 0); a lapsed maintenance window boots with a warning
//! (perpetual license — support/updates lapse, the app keeps running).
//!
//! License file format (JSON):
//!
//! ```json
//! {"payload": {"org": "...", "edition": "team", "issued": "YYYY-MM-DD",
//!              "maintenance_until": "YYYY-MM-DD", "key_id": "..."},
//!  "signature": "<base64 ed25519 over canonical JSON of payload>"}
//! ```
//!
//! Canonical form is Python `json.dumps(payload, sort_keys=True,
//! separators=(",", ":"))` — sorted keys, no spaces, non-ASCII escaped as
//! `\uXXXX` (surrogate pairs above U+FFFF). [`py_canonical`] reproduces that
//! byte-for-byte so signatures made by `make_license.py` verify here.

use base64::Engine as _;
use chrono::NaiveDate;
use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::{Map, Value};

pub const TEAM_EDITION: &str = "team";

/// Ed25519 public key(s), raw 32-byte hex. Mirrors `PUBLIC_KEYS_HEX` in the
/// frozen `backend/app/license.py`. The private half lives with the seller
/// only — it must never be committed. Rotate by appending and matching
/// `key_id` prefixes in `make_license.py`.
pub const PUBLIC_KEYS_HEX: &[&str] =
    &["37c9b28f28948a06b7d3a9b9b32b799fd6f4a018ccaaffb12ad89950e1b0ca3e"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseStatus {
    Ok,
    Lapsed,
    Missing,
    Invalid,
}

#[derive(Debug, Clone)]
pub struct LicenseResult {
    pub status: LicenseStatus,
    pub org: Option<String>,
    pub maintenance_until: Option<String>,
    pub message: String,
}

fn invalid(msg: impl Into<String>) -> LicenseResult {
    LicenseResult {
        status: LicenseStatus::Invalid,
        org: None,
        maintenance_until: None,
        message: msg.into(),
    }
}

/// Python `json.dumps(s, ensure_ascii=True)` string escaping: `"` and `\`
/// special-cased, `\b \t \n \f \r` shortcuts, `\uXXXX` for control chars and
/// everything above a tilde, surrogate pairs above U+FFFF.
fn py_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{09}' => out.push_str("\\t"),
            '\u{0a}' => out.push_str("\\n"),
            '\u{0c}' => out.push_str("\\f"),
            '\u{0d}' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) > 0x7e => {
                let cp = c as u32;
                if cp > 0xffff {
                    let v = cp - 0x10000;
                    out.push_str(&format!(
                        "\\u{:04x}\\u{:04x}",
                        0xd800 + (v >> 10),
                        0xdc00 + (v & 0x3ff)
                    ));
                } else {
                    out.push_str(&format!("\\u{:04x}", cp));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Canonical bytes of a flat string map exactly like the signed Python form.
fn py_canonical(payload: &Map<String, Value>) -> Result<Vec<u8>, String> {
    let mut keys: Vec<&String> = payload.keys().collect();
    keys.sort();
    let mut out = String::from("{");
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let v = payload[*k]
            .as_str()
            .ok_or_else(|| format!("non-string payload field {k}"))?;
        out.push_str(&py_json_string(k));
        out.push(':');
        out.push_str(&py_json_string(v));
    }
    out.push('}');
    Ok(out.into_bytes())
}

fn decode_key(hex_str: &str) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = hex::decode(hex_str).ok()?.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

fn any_key_verifies(canonical: &[u8], sig: &Signature, keys: &[&str]) -> bool {
    keys.iter()
        .filter_map(|h| decode_key(h))
        .any(|vk| vk.verify_strict(canonical, sig).is_ok())
}

/// Parse the license document; refuses non-team editions and bad signatures.
fn parse_doc(raw: &str) -> Result<(Map<String, Value>, Signature), String> {
    let doc: Value = serde_json::from_str(raw).map_err(|e| format!("malformed license: {e}"))?;
    let payload = doc
        .get("payload")
        .and_then(|v| v.as_object())
        .ok_or("malformed license: missing payload")?
        .clone();
    if payload.get("edition").and_then(|v| v.as_str()) != Some(TEAM_EDITION) {
        return Err("malformed license: not a team license".into());
    }
    let sig_b64 = doc
        .get("signature")
        .and_then(|v| v.as_str())
        .ok_or("malformed license: missing signature")?;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(sig_b64)
        .map_err(|e| format!("malformed license: bad signature encoding: {e}"))?;
    let sig = Signature::from_slice(&sig_bytes)
        .map_err(|e| format!("malformed license: bad signature: {e}"))?;
    Ok((payload, sig))
}

/// Verify a license document. Pure function — no env, no I/O.
pub fn verify_license(
    raw: Option<&str>,
    public_keys_hex: Option<&[&str]>,
    today: Option<NaiveDate>,
) -> LicenseResult {
    let day = today.unwrap_or_else(|| chrono::Utc::now().date_naive());
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return LicenseResult {
            status: LicenseStatus::Missing,
            org: None,
            maintenance_until: None,
            message: "no license provided".into(),
        };
    };
    let (payload, sig) = match parse_doc(raw) {
        Ok(v) => v,
        Err(e) => return invalid(e),
    };
    let until = match payload
        .get("maintenance_until")
        .and_then(|v| v.as_str())
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
    {
        Some(d) => d,
        None => return invalid("malformed license: bad maintenance_until"),
    };
    let org = payload
        .get("org")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let canonical = match py_canonical(&payload) {
        Ok(c) => c,
        Err(e) => return invalid(e),
    };
    let keys = public_keys_hex.unwrap_or(PUBLIC_KEYS_HEX);
    if keys.is_empty() {
        return invalid("no valid public key configured");
    }
    if !any_key_verifies(&canonical, &sig, keys) {
        return LicenseResult {
            status: LicenseStatus::Invalid,
            org: Some(org),
            maintenance_until: Some(until.to_string()),
            message: "signature mismatch".into(),
        };
    }
    if until < day {
        return LicenseResult {
            status: LicenseStatus::Lapsed,
            org: Some(org),
            maintenance_until: Some(until.to_string()),
            message: format!("maintenance expired {until} — app runs, support/updates lapsed"),
        };
    }
    let message = format!("licensed to {org} until {until}");
    LicenseResult {
        status: LicenseStatus::Ok,
        org: Some(org),
        maintenance_until: Some(until.to_string()),
        message,
    }
}

/// Read from `ANCHOR_LICENSE_FILE` (path) or `ANCHOR_LICENSE` (inline JSON).
pub fn load_license_from_env() -> LicenseResult {
    if let Ok(path) = std::env::var("ANCHOR_LICENSE_FILE") {
        if !path.trim().is_empty() {
            return match std::fs::read_to_string(&path) {
                Ok(raw) => verify_license(Some(&raw), None, None),
                Err(e) => LicenseResult {
                    status: LicenseStatus::Missing,
                    org: None,
                    maintenance_until: None,
                    message: format!("cannot read {path}: {e}"),
                },
            };
        }
    }
    verify_license(std::env::var("ANCHOR_LICENSE").ok().as_deref(), None, None)
}

/// Startup gate for the Team edition. `Err` means: refuse to boot.
pub fn enforce_team_license() -> Result<LicenseResult, String> {
    let result = load_license_from_env();
    match result.status {
        LicenseStatus::Ok => {
            tracing::info!("Team license: {}", result.message);
            Ok(result)
        }
        LicenseStatus::Lapsed => {
            tracing::warn!("Team license lapsed: {}", result.message);
            Ok(result)
        }
        _ => Err(format!(
            "Team edition requires a license ({}). Mount it at ANCHOR_LICENSE_FILE or set \
             ANCHOR_LICENSE. Contact sales at alex@anchorcore.dev for a platform license.",
            result.message
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Throwaway keypair used only to produce these fixtures with the real
    // issuer (`python scripts/make_license.py`); not the production key.
    const TEST_PUB: &str = "ac7dbe211334819eb697b35f8aec9c9ea32015eb96dca7cb36adaa1c85b50a72";

    // Issued with `--org "B59 T\u00ebst \ud83d\ude80 SAS"` (non-ASCII + surrogate pair).
    const VALID: &str = r#"{"payload": {"org": "B59 T\u00ebst \ud83d\ude80 SAS", "edition": "team", "issued": "2026-10-10", "maintenance_until": "2027-10-10", "key_id": "ac7dbe21"}, "signature": "fHlWcVaIFWsPzW97ye+snoTgotBdeL03zUb9ifG3IwmEKKv93u1DkxOiD3go5qrBYbyu7NXm6Ttf72MjRac8DQ=="}"#;
    const LAPSED: &str = r#"{"payload": {"org": "B59 T\u00ebst \ud83d\ude80 SAS", "edition": "team", "issued": "2026-10-10", "maintenance_until": "2025-10-10", "key_id": "ac7dbe21"}, "signature": "N6JWHaazyGDWWdGPUgdO0vdryg3y3BzXsmJIONxm5wME7eVcdysWHuWiPq3wRP7cZNfM/XX4QLM8+cOM9Q/CCw=="}"#;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn python_canonical_matches_issuer_bytes() {
        let (payload, _sig) = parse_doc(VALID).unwrap();
        let canonical = String::from_utf8(py_canonical(&payload).unwrap()).unwrap();
        assert_eq!(
            canonical,
            r#"{"edition":"team","issued":"2026-10-10","key_id":"ac7dbe21","maintenance_until":"2027-10-10","org":"B59 T\u00ebst \ud83d\ude80 SAS"}"#
        );
    }

    #[test]
    fn python_string_escaping() {
        assert_eq!(
            py_json_string("B59 T\u{eb}st \u{1f680} SAS"),
            r#""B59 T\u00ebst \ud83d\ude80 SAS""#
        );
        assert_eq!(
            py_json_string("\"\n\t\\\u{7f}\u{e9}"),
            r#""\"\n\t\\\u007f\u00e9""#
        );
        assert_eq!(py_json_string("\u{08}\u{0c}\u{0d}"), r#""\b\f\r""#);
    }

    #[test]
    fn valid_license_ok() {
        let r = verify_license(Some(VALID), Some(&[TEST_PUB]), Some(day(2026, 10, 10)));
        assert_eq!(r.status, LicenseStatus::Ok);
        assert_eq!(r.org.as_deref(), Some("B59 T\u{eb}st \u{1f680} SAS"));
        assert_eq!(r.maintenance_until.as_deref(), Some("2027-10-10"));
    }

    #[test]
    fn lapsed_after_maintenance_date() {
        let r = verify_license(Some(VALID), Some(&[TEST_PUB]), Some(day(2028, 1, 1)));
        assert_eq!(r.status, LicenseStatus::Lapsed);
    }

    #[test]
    fn lapsed_fixture_within_original_window() {
        let r = verify_license(Some(LAPSED), Some(&[TEST_PUB]), Some(day(2026, 10, 10)));
        assert_eq!(r.status, LicenseStatus::Lapsed);
    }

    #[test]
    fn tampered_signature_invalid() {
        let tampered = format!("{}x", &VALID[..VALID.len() - 2]);
        let r = verify_license(Some(&tampered), Some(&[TEST_PUB]), Some(day(2026, 10, 10)));
        assert_eq!(r.status, LicenseStatus::Invalid);
    }

    #[test]
    fn wrong_key_invalid() {
        let other = "ac7dbe211334819eb697b35f8aec9c9ea32015eb96dca7cb36adaa1c85b50a70";
        let r = verify_license(Some(VALID), Some(&[other]), Some(day(2026, 10, 10)));
        assert_eq!(r.status, LicenseStatus::Invalid);
    }

    #[test]
    fn missing_or_blank_is_missing() {
        for raw in [None, Some(""), Some("   ")] {
            let r = verify_license(raw, Some(&[TEST_PUB]), Some(day(2026, 10, 10)));
            assert_eq!(r.status, LicenseStatus::Missing);
        }
    }

    #[test]
    fn personal_edition_document_rejected() {
        let doc = VALID.replace("\"team\"", "\"personal\"");
        let r = verify_license(Some(&doc), Some(&[TEST_PUB]), Some(day(2026, 10, 10)));
        assert_eq!(r.status, LicenseStatus::Invalid);
    }
}
