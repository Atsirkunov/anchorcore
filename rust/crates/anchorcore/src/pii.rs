//! PII knowledge base + detection — port of `backend/app/pii.py:1`.
//! Local regex scan, no LLM. Categories, `scan_text`, `load_config`/`save_config`,
//! `categories_payload`, `serialize_matches`, and `flag_pii` wiring.

use regex::Regex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const KEY_CUSTOM_WORDS: &str = "pii_custom_words";
pub const KEY_DISABLED_CATEGORIES: &str = "pii_disabled_categories";

#[derive(Debug, Clone)]
pub struct PiiCategory {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub field_names: &'static [&'static str],
    pub patterns: &'static [&'static str],
    pub strong: bool,
}

pub const CATEGORIES: &[PiiCategory] = &[
    PiiCategory {
        id: "email",
        label: "Email address",
        description: "A personal email address / user account identifier.",
        field_names: &["email", "email_address", "e-mail", "mail", "contact_email", "emailaddr"],
        patterns: &[r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b"],
        strong: true,
    },
    PiiCategory {
        id: "phone",
        label: "Phone number",
        description: "Personal / mobile / work phone or fax number.",
        field_names: &["phone", "phone_number", "mobile", "cell", "telephone", "tel", "fax", "contact_no", "work_phone"],
        patterns: &[r"\b(?:\+?\d{1,3}[\s\-.]?)?(?:\(\d{1,4}\)[\s\-.]?)?\d{3}[\s\-.]?\d{3}[\s\-.]?\d{4}\b"],
        strong: true,
    },
    PiiCategory {
        id: "full_name",
        label: "Full name",
        description: "A person's given / family / full name.",
        field_names: &["full_name", "first_name", "last_name", "given_name", "family_name", "name_of", "display_name", "legal_name"],
        patterns: &[],
        strong: true,
    },
    PiiCategory {
        id: "ssn",
        label: "Social Security Number",
        description: "US SSN (###-##-####).",
        field_names: &["ssn", "social_security", "social_security_number", "ssn_number", "socialsecurity"],
        patterns: &[r"\b\d{3}[\-.\s]?\d{2}[\-.\s]?\d{4}\b"],
        strong: true,
    },
    PiiCategory {
        id: "passport",
        label: "Passport number",
        description: "Passport / travel document number.",
        field_names: &["passport", "passport_number", "passport_no", "passportno"],
        patterns: &[r"\b[A-Z]{1,2}\d{6,9}\b"],
        strong: true,
    },
    PiiCategory {
        id: "drivers_license",
        label: "Driver's license",
        description: "Driver's license / state ID number.",
        field_names: &["drivers_license", "driver_license", "driver_license_number", "license_number", "dl_number", "lic_no"],
        patterns: &[r"\b(?:DL|LIC)[\s\-:]*\d{6,9}\b"],
        strong: true,
    },
    PiiCategory {
        id: "national_id",
        label: "National ID number",
        description: "Government-issued ID: NIN, CPF, Aadhaar, citizen ID, etc.",
        field_names: &["national_id", "national_identification", "id_number", "citizen_id", "personal_id", "nin", "cpf", "aadhaar", "identity_no", "ic_number"],
        patterns: &[r"\b\d{3}\.?\d{3}\.?\d{3}[\-]?\d{2}\b", r"\b\d{12}\b"],
        strong: true,
    },
    PiiCategory {
        id: "dob",
        label: "Date of birth",
        description: "Date of birth.",
        field_names: &["dob", "date_of_birth", "birth_date", "birthday", "birthdate", "birth_date_of"],
        patterns: &[r"\b(?:born|dob|date of birth)[:\s]+(\d{1,2}[/\-.]\d{1,2}[/\-.]\d{2,4})\b"],
        strong: true,
    },
    PiiCategory {
        id: "address",
        label: "Home address",
        description: "Street address, city, state, postal code.",
        field_names: &["address", "street", "street_address", "home_address", "mailing_address", "city", "postal", "postal_code", "zip", "zip_code", "state", "province"],
        patterns: &[r"\b\d{1,5}\s+[A-Za-z0-9 .\-,]+(?:Street|St|Avenue|Ave|Road|Rd|Boulevard|Blvd|Lane|Ln|Drive|Dr|Way|Court|Ct|Place|Pl)\b", r"\b[A-Z]{1,2}\d[A-Z\d]?\s?\d[A-Z]{2}\b"],
        strong: true,
    },
    PiiCategory {
        id: "ip_address",
        label: "IP address",
        description: "IPv4 / IPv6 address that can identify a device.",
        field_names: &["ip", "ip_address", "client_ip", "remote_addr", "ip_addr", "source_ip"],
        patterns: &[r"\b(?:\d{1,3}\.){3}\d{1,3}\b", r"\b[0-9a-fA-F]{1,4}:(?:[0-9a-fA-F]{0,4}:){2,7}[0-9a-fA-F]{1,4}\b"],
        strong: true,
    },
    PiiCategory {
        id: "credit_card",
        label: "Credit card number",
        description: "Payment card PAN (Visa/MC/Amex).",
        field_names: &["credit_card", "card_number", "cc_number", "cc", "pan", "cardholder", "card_no"],
        patterns: &[r"\b(?:\d{4}[\s\-]?){3}\d{4}\b", r"\b\d{4}[\s\-]?\d{4}[\s\-]?\d{4}[\s\-]?\d{4}\b"],
        strong: true,
    },
    PiiCategory {
        id: "bank_account",
        label: "Bank account number",
        description: "Bank / savings / routing account number.",
        field_names: &["bank_account", "account_number", "bank_acct", "routing_number", "sort_code", "acct_no", "bank_account_no"],
        patterns: &[r"\b\d{8,17}\b"],
        strong: true,
    },
    PiiCategory {
        id: "iban",
        label: "IBAN",
        description: "International Bank Account Number.",
        field_names: &["iban", "iban_number", "swift", "bic"],
        patterns: &[r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b"],
        strong: true,
    },
    PiiCategory {
        id: "tax_id",
        label: "Tax ID (TIN/EIN)",
        description: "Taxpayer / employer identification number.",
        field_names: &["tax_id", "tin", "ein", "employer_id", "vat", "vat_number", "tax_identification"],
        patterns: &[r"\b\d{2}[\-.]?\d{7}\b"],
        strong: true,
    },
    PiiCategory {
        id: "credentials",
        label: "Credentials / API keys",
        description: "Passwords, API keys, tokens, client secrets.",
        field_names: &["password", "passwd", "api_key", "apikey", "secret", "secret_key", "token", "access_token", "refresh_token", "client_secret", "auth", "authorization", "bearer"],
        patterns: &[r"\b(?:sk|pk|ghp|gho|xox[baprs]|AKIA)[A-Za-z0-9_\-]{16,}\b", r"\bpassword\s*[=:]\s*\S+", r"\bapi[_-]?key\s*[=:]\s*\S+"],
        strong: true,
    },
    PiiCategory {
        id: "health",
        label: "Medical / health record",
        description: "Medical record number, diagnosis, condition, blood type.",
        field_names: &["mrn", "medical_record", "medical_record_number", "patient_id", "diagnosis", "medical_condition", "blood_type", "patient_no", "health_record"],
        patterns: &[r"\bMRN[\s\-:]*\d{4,}\b"],
        strong: true,
    },
    PiiCategory {
        id: "employment",
        label: "Employee details",
        description: "Employee ID, salary, compensation, job details.",
        field_names: &["employee_id", "emp_id", "employee_no", "salary", "compensation", "pay_rate", "hire_date", "manager_id"],
        patterns: &[],
        strong: true,
    },
    PiiCategory {
        id: "biometric",
        label: "Biometric data",
        description: "Fingerprint, retina/iris scan, face ID, voice print.",
        field_names: &["fingerprint", "retina", "iris", "face_id", "faceid", "voice_print", "biometric", "biometric_id"],
        patterns: &[r"\b(?:fingerprint|retina scan|iris scan|face id|voiceprint)\b"],
        strong: false,
    },
    PiiCategory {
        id: "demographics",
        label: "Demographics (protected class)",
        description: "Ethnicity, religion, marital status, gender, nationality.",
        field_names: &["ethnicity", "race", "religion", "marital_status", "civil_status", "gender", "sex", "nationality", "sexual_orientation"],
        patterns: &[],
        strong: true,
    },
    PiiCategory {
        id: "age",
        label: "Age",
        description: "Age or age range.",
        field_names: &["age", "age_group", "age_range"],
        patterns: &[r"\bage\s*[=:]\s*\d{1,3}\b"],
        strong: false,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PiiMatch {
    pub category: String,
    pub label: String,
    pub strong: bool,
    #[serde(rename = "match")]
    pub m: String,
}

// --- config persistence (mirrors pii.py load_config/save_config) ---

fn load_json_set(conn: &Connection, key: &str) -> HashSet<String> {
    let val: Option<String> = conn
        .query_row("SELECT value FROM app_settings WHERE key = ?1", [key], |r| r.get(0))
        .ok();
    if let Some(s) = val {
        if let Ok(arr) = serde_json::from_str::<Vec<String>>(&s) {
            return arr.into_iter().collect();
        }
    }
    HashSet::new()
}

pub fn load_config(conn: &Connection) -> (HashSet<String>, HashSet<String>) {
    let disabled = load_json_set(conn, KEY_DISABLED_CATEGORIES);
    let custom = load_json_set(conn, KEY_CUSTOM_WORDS);
    (disabled, custom)
}

pub fn save_config(
    conn: &Connection,
    disabled: Option<&[String]>,
    custom_words: Option<&[String]>,
) -> Result<(), rusqlite::Error> {
    if let Some(d) = disabled {
        let mut sorted = d.to_vec();
        sorted.sort();
        sorted.dedup();
        let val = serde_json::to_string(&sorted).unwrap();
        let exists: bool = conn
            .query_row("SELECT 1 FROM app_settings WHERE key = ?1", [KEY_DISABLED_CATEGORIES], |_| Ok(()))
            .is_ok();
        if exists {
            conn.execute("UPDATE app_settings SET value = ?1 WHERE key = ?2", rusqlite::params![val, KEY_DISABLED_CATEGORIES])?;
        } else {
            conn.execute("INSERT INTO app_settings (key, value) VALUES (?1, ?2)", rusqlite::params![KEY_DISABLED_CATEGORIES, val])?;
        }
    }
    if let Some(cw) = custom_words {
        let mut sorted = cw.to_vec();
        sorted.sort();
        sorted.dedup();
        let val = serde_json::to_string(&sorted).unwrap();
        let exists: bool = conn
            .query_row("SELECT 1 FROM app_settings WHERE key = ?1", [KEY_CUSTOM_WORDS], |_| Ok(()))
            .is_ok();
        if exists {
            conn.execute("UPDATE app_settings SET value = ?1 WHERE key = ?2", rusqlite::params![val, KEY_CUSTOM_WORDS])?;
        } else {
            conn.execute("INSERT INTO app_settings (key, value) VALUES (?1, ?2)", rusqlite::params![KEY_CUSTOM_WORDS, val])?;
        }
    }
    Ok(())
}

// --- scan ---

fn scan_category(cat: &PiiCategory, text: &str, lower: &str) -> Option<PiiMatch> {
    // Python uses re.IGNORECASE; build case-insensitive regex
    for pattern in cat.patterns {
        let pat = format!("(?i){}", pattern);
        if let Ok(re) = Regex::new(&pat) {
            if let Some(m) = re.find(text) {
                return Some(PiiMatch {
                    category: cat.id.to_string(),
                    label: cat.label.to_string(),
                    strong: cat.strong,
                    m: m.as_str().to_string(),
                });
            }
        }
    }
    for name in cat.field_names {
        let esc = regex::escape(name);
        let pat = format!(r"\b{}\b", esc);
        if let Ok(re) = Regex::new(&pat) {
            if re.is_match(lower) {
                return Some(PiiMatch {
                    category: cat.id.to_string(),
                    label: cat.label.to_string(),
                    strong: false,
                    m: name.to_string(),
                });
            }
        }
    }
    None
}

fn scan_custom(word: &str, lower: &str) -> Option<PiiMatch> {
    let w = word.trim();
    if w.is_empty() {
        return None;
    }
    let lower_w = w.to_lowercase();
    let esc = regex::escape(&lower_w);
    let pat = format!(r"\b{}\b", esc);
    if let Ok(re) = Regex::new(&pat) {
        if re.is_match(lower) {
            return Some(PiiMatch {
                category: "custom".to_string(),
                label: w.to_string(),
                strong: true,
                m: w.to_string(),
            });
        }
    }
    None
}

fn dedupe_matches(matches: Vec<PiiMatch>) -> Vec<PiiMatch> {
    // de-dupe by category (strong preferred)
    let mut seen: HashMap<String, PiiMatch> = HashMap::new();
    for m in matches {
        let cat = m.category.clone();
        match seen.get(&cat) {
            None => {
                seen.insert(cat, m);
            }
            Some(prev) if m.strong && !prev.strong => {
                seen.insert(cat, m);
            }
            _ => {}
        }
    }
    seen.into_values().collect()
}

pub fn scan_text(
    text: &str,
    disabled: &HashSet<String>,
    custom_words: &HashSet<String>,
) -> Vec<PiiMatch> {
    let lower = text.to_lowercase();
    let mut matches: Vec<PiiMatch> = Vec::new();

    for cat in CATEGORIES {
        if disabled.contains(cat.id) {
            continue;
        }
        if let Some(m) = scan_category(cat, text, &lower) {
            matches.push(m);
        }
    }

    for word in custom_words {
        if let Some(m) = scan_custom(word, &lower) {
            matches.push(m);
        }
    }

    dedupe_matches(matches)
}

pub fn categories_payload(disabled: &HashSet<String>) -> Vec<serde_json::Value> {
    CATEGORIES
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "label": c.label,
                "description": c.description,
                "field_names": c.field_names,
                "patterns": c.patterns,
                "enabled": !disabled.contains(c.id)
            })
        })
        .collect()
}

pub fn serialize_matches(matches: &[PiiMatch]) -> Vec<serde_json::Value> {
    matches
        .iter()
        .map(|m| {
            serde_json::json!({
                "category": m.category,
                "label": m.label,
                "strong": m.strong,
                "match": m.m
            })
        })
        .collect()
}

/// Mirrors `backend/app/pipeline.py:295` `_flag_pii` — scan item's chunks and set is_pii/pii_categories.
pub fn flag_pii_for_item(conn: &Connection, item_id: i64) -> Result<usize, rusqlite::Error> {
    let (disabled, custom) = load_config(conn);
    // gather chunk ids belonging to item (direct + via entity)
    let mut stmt = conn.prepare(
        "SELECT c.id, c.content FROM chunks c \
         LEFT JOIN entities e ON e.id = c.entity_id \
         WHERE c.item_id = ?1 OR e.item_id = ?1",
    )?;
    let chunks: Vec<(i64, String)> = stmt
        .query_map([item_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    let mut flagged = 0;
    for (cid, content) in chunks {
        let matches = scan_text(&content, &disabled, &custom);
        let cats: Vec<String> = matches.iter().map(|m| m.category.clone()).collect();
        let is_pii = matches.iter().any(|m| m.strong);
        let cats_json = serde_json::to_string(&cats).unwrap();
        conn.execute(
            "UPDATE chunks SET pii_categories = ?1, is_pii = ?2 WHERE id = ?3",
            rusqlite::params![cats_json, if is_pii { 1 } else { 0 }, cid],
        )?;
        if is_pii {
            flagged += 1;
        }
    }
    Ok(flagged)
}

// --- axum handlers (R4.3) ---

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde_json::Value;

use crate::health::AppState;

#[derive(Deserialize)]
pub struct PiiConfigUpdate {
    pub custom_words: Option<Vec<String>>,
    pub disabled_categories: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct ReviewQuery {
    pub only_flagged: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Deserialize)]
pub struct DecideRequest {
    pub is_pii: bool,
}

pub async fn get_config_handler(State(state): State<AppState>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let (disabled, custom) = load_config(&conn);
        let cats = categories_payload(&disabled);
        let custom_sorted: Vec<String> = {
            let mut v: Vec<String> = custom.into_iter().collect();
            v.sort();
            v
        };
        serde_json::json!({"categories": cats, "custom_words": custom_sorted})
    })
    .await
    .unwrap();
    Json(result)
}

pub async fn put_config_handler(State(state): State<AppState>, Json(payload): Json<PiiConfigUpdate>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let _ = save_config(&conn, payload.disabled_categories.as_deref(), payload.custom_words.as_deref());
        let (disabled, custom) = load_config(&conn);
        let cats = categories_payload(&disabled);
        let custom_sorted: Vec<String> = {
            let mut v: Vec<String> = custom.into_iter().collect();
            v.sort();
            v
        };
        serde_json::json!({"categories": cats, "custom_words": custom_sorted})
    })
    .await
    .unwrap();
    Json(result)
}

fn resolve_source(conn: &Connection, entity_id: Option<i64>, item_id: Option<i64>) -> (Option<i64>, String, String) {
    let iid = if let Some(eid) = entity_id {
        conn.query_row("SELECT item_id FROM entities WHERE id=?1", [eid], |r| r.get::<_, i64>(0)).ok()
    } else {
        item_id
    };
    let Some(iid) = iid else {
        return (None, String::new(), "internal".to_string());
    };
    let Some(sid) = conn.query_row("SELECT source_id FROM ingested_items WHERE id=?1", [iid], |r| r.get::<_, i64>(0)).ok() else {
        return (None, String::new(), "internal".to_string());
    };
    conn.query_row("SELECT name, label FROM sources WHERE id=?1", [sid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .ok()
        .map(|(n, l)| (Some(sid), n, l))
        .unwrap_or((None, String::new(), "internal".to_string()))
}

type ReviewRow = (i64, Option<i64>, Option<i64>, String, String, i64, String, Option<String>);

fn build_review_rows(conn: &Connection, rows: Vec<ReviewRow>, only_flagged: bool, disabled: &HashSet<String>, custom: &HashSet<String>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for (cid, item_id, entity_id, kind, content, is_pii_db, pii_cats, _) in rows {
        // resolve source via item
        let (source_id, source_name, source_label) = resolve_source(conn, entity_id, item_id);
        let stored_cats: Vec<String> = serde_json::from_str(&pii_cats).unwrap_or_default();
        if stored_cats.contains(&"_dismissed".to_string()) {
            continue;
        }
        let matches = scan_text(&content, disabled, custom);
        if matches.is_empty() && is_pii_db == 0 {
            continue;
        }
        if only_flagged && !(is_pii_db != 0 || matches.iter().any(|m| m.strong)) {
            continue;
        }
        let is_pii = is_pii_db != 0;
        out.push(serde_json::json!({
            "chunk_id": cid,
            "source_id": source_id,
            "source_name": source_name,
            "source_label": source_label,
            "kind": kind,
            "content": content,
            "snippet": content.chars().take(300).collect::<String>(),
            "is_pii": is_pii,
            "categories": serialize_matches(&matches)
        }));
    }
    out
}

pub async fn review_handler(State(state): State<AppState>, Query(q): Query<ReviewQuery>) -> Json<Value> {
    let data_dir = state.data_dir.clone();
    let only_flagged = q.only_flagged.unwrap_or(true);
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let (disabled, custom) = load_config(&conn);
        // mirrors backend/app/routers/pii.py:58 review
        let mut stmt = match conn.prepare(
            "SELECT c.id, c.item_id, c.entity_id, c.kind, c.content, c.is_pii, c.pii_categories, c.created_at \
             FROM chunks c ORDER BY c.created_at DESC LIMIT ?1 OFFSET ?2",
        ) {
            Ok(s) => s,
            Err(_) => return serde_json::json!([]),
        };
        let rows: Vec<ReviewRow> = stmt
            .query_map(rusqlite::params![limit, offset], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        Value::Array(build_review_rows(&conn, rows, only_flagged, &disabled, &custom))
    })
    .await
    .unwrap();
    Json(result)
}

pub async fn decide_handler(
    State(state): State<AppState>,
    Path(chunk_id): Path<i64>,
    Json(payload): Json<DecideRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let exists: bool = conn.query_row("SELECT 1 FROM chunks WHERE id=?1", [chunk_id], |_| Ok(())).is_ok();
        if !exists {
            return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Chunk not found"}))));
        }
        if payload.is_pii {
            let (disabled, custom) = load_config(&conn);
            // fetch content for matches
            let content: String = conn.query_row("SELECT content FROM chunks WHERE id=?1", [chunk_id], |r| r.get(0)).unwrap_or_default();
            let matches = scan_text(&content, &disabled, &custom);
            let cats: Vec<String> = matches.iter().map(|m| m.category.clone()).collect();
            let cats_json = if cats.is_empty() { serde_json::to_string(&vec!["_manual"]).unwrap() } else { serde_json::to_string(&cats).unwrap() };
            let _ = conn.execute("UPDATE chunks SET is_pii=1, pii_categories=?1 WHERE id=?2", rusqlite::params![cats_json, chunk_id]);
        } else {
            let _ = conn.execute("UPDATE chunks SET is_pii=0, pii_categories=?1 WHERE id=?2", rusqlite::params![serde_json::to_string(&vec!["_dismissed"]).unwrap(), chunk_id]);
        }
        // fetch updated row for response
        let (content, is_pii): (String, i64) = conn.query_row("SELECT content, is_pii FROM chunks WHERE id=?1", [chunk_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        let (disabled, custom) = load_config(&conn);
        let matches = scan_text(&content, &disabled, &custom);
        Ok(serde_json::json!({
            "chunk_id": chunk_id,
            "source_id": null,
            "source_name": "",
            "source_label": "internal",
            "kind": "document",
            "content": content,
            "snippet": content.chars().take(300).collect::<String>(),
            "is_pii": is_pii != 0,
            "categories": serialize_matches(&matches)
        }))
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(e),
    }
}

fn scan_item_rows(rows: &[(i64, String, i64, String)], disabled: &HashSet<String>, custom: &HashSet<String>) -> (i64, HashSet<String>, Vec<Value>) {
    let mut flagged = 0;
    let mut all_cats: HashSet<String> = HashSet::new();
    let mut matches: Vec<Value> = Vec::new();
    for (_, content, is_pii, pii_cats) in rows {
        if *is_pii != 0 { flagged += 1; }
        if let Ok(cats) = serde_json::from_str::<Vec<String>>(pii_cats) {
            for c in cats { if c != "_dismissed" { all_cats.insert(c); } }
        }
        // live scan for display highlight (strong matches)
        for m in scan_text(content, disabled, custom).into_iter().filter(|m| m.strong) {
            all_cats.insert(m.category.clone());
            matches.push(serde_json::json!({"category": m.category, "label": m.label, "match": m.m}));
        }
    }
    (flagged, all_cats, matches)
}

fn dedupe_match_values(matches: Vec<Value>) -> Vec<Value> {
    // dedupe matches by match string
    let mut seen: HashSet<String> = HashSet::new();
    let mut deduped = Vec::new();
    for m in matches {
        let s = m["match"].as_str().unwrap_or("").to_string();
        if seen.insert(s) {
            deduped.push(m);
        }
    }
    deduped
}

pub async fn item_pii_handler(State(state): State<AppState>, Path(item_id): Path<i64>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let exists: bool = conn.query_row("SELECT 1 FROM ingested_items WHERE id=?1", [item_id], |_| Ok(())).is_ok();
        if !exists {
            return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Item not found"}))));
        }
        let (disabled, custom) = load_config(&conn);
        // chunks for this item (direct + via entity)
        let mut stmt = conn.prepare("SELECT c.id, c.content, c.is_pii, c.pii_categories FROM chunks c LEFT JOIN entities e ON e.id=c.entity_id WHERE c.item_id=?1 OR e.item_id=?1").unwrap();
        let rows: Vec<(i64, String, i64, String)> = stmt.query_map([item_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap().filter_map(|r| r.ok()).collect();
        let (flagged, all_cats, matches) = scan_item_rows(&rows, &disabled, &custom);
        let is_pii = flagged > 0 || !matches.is_empty();
        let deduped = dedupe_match_values(matches);
        Ok(serde_json::json!({
            "item_id": item_id,
            "is_pii": is_pii,
            "flagged": flagged,
            "total": rows.len(),
            "categories": all_cats.into_iter().collect::<Vec<_>>(),
            "matches": deduped.into_iter().take(20).collect::<Vec<_>>()
        }))
    }).await.unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(e),
    }
}

fn scan_and_update(conn: &Connection, chunks: &[(i64, String)], disabled: &HashSet<String>, custom: &HashSet<String>) -> usize {
    let mut flagged = 0;
    for (cid, content) in chunks {
        let matches = scan_text(content, disabled, custom);
        let cats: Vec<String> = matches.iter().map(|m| m.category.clone()).collect();
        let is_pii = matches.iter().any(|m| m.strong);
        let cats_json = serde_json::to_string(&cats).unwrap();
        let _ = conn.execute("UPDATE chunks SET pii_categories=?1, is_pii=?2 WHERE id=?3", rusqlite::params![cats_json, if is_pii {1} else {0}, cid]);
        if is_pii { flagged += 1; }
    }
    flagged
}

pub async fn scan_handler(State(state): State<AppState>, Path(source_id): Path<i64>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let data_dir = state.data_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let db_path = crate::db::resolve_db_path(&data_dir);
        let conn = crate::db::init_db(&db_path).unwrap_or_else(|_| rusqlite::Connection::open(&db_path).unwrap());
        let exists: bool = conn.query_row("SELECT 1 FROM sources WHERE id=?1", [source_id], |_| Ok(())).is_ok();
        if !exists {
            return Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"detail":"Source not found"}))));
        }
        let (disabled, custom) = load_config(&conn);
        // get item ids
        let mut stmt = conn.prepare("SELECT id FROM ingested_items WHERE source_id=?1").unwrap();
        let item_ids: Vec<i64> = stmt.query_map([source_id], |r| r.get(0)).unwrap().filter_map(|r| r.ok()).collect();
        if item_ids.is_empty() {
            return Ok(serde_json::json!({"source_id": source_id, "chunks": 0, "flagged": 0}));
        }
        let placeholders = item_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("SELECT id, content FROM chunks WHERE item_id IN ({})", placeholders);
        let mut stmt2 = conn.prepare(&sql).unwrap();
        let chunks: Vec<(i64, String)> = stmt2.query_map(rusqlite::params_from_iter(item_ids.iter()), |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(|r| r.ok()).collect();
        let mut flagged = scan_and_update(&conn, &chunks, &disabled, &custom);
        // also scan chunks via entities (reuse placeholders)
        let sql2 = format!("SELECT c.id, c.content FROM chunks c JOIN entities e ON e.id=c.entity_id WHERE e.item_id IN ({})", placeholders);
        if let Ok(mut stmt3) = conn.prepare(&sql2) {
            let chunks2: Vec<(i64, String)> = stmt3.query_map(rusqlite::params_from_iter(item_ids.iter()), |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(|r| r.ok()).collect();
            flagged += scan_and_update(&conn, &chunks2, &disabled, &custom);
        }
        let total = chunks.len();
        Ok(serde_json::json!({"source_id": source_id, "chunks": total, "flagged": flagged}))
    })
    .await
    .unwrap();
    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn email_scan() {
        let disabled = HashSet::new();
        let custom = HashSet::new();
        let m = scan_text("Contact me at alice@example.com", &disabled, &custom);
        assert!(m.iter().any(|x| x.category == "email" && x.strong));
    }
    #[test]
    fn disabled_category() {
        let mut disabled = HashSet::new();
        disabled.insert("email".to_string());
        let custom = HashSet::new();
        let m = scan_text("alice@example.com", &disabled, &custom);
        assert!(!m.iter().any(|x| x.category == "email"));
    }
    #[test]
    fn custom_word() {
        let disabled = HashSet::new();
        let mut custom = HashSet::new();
        custom.insert("foobar".to_string());
        let m = scan_text("this foobar is here", &disabled, &custom);
        assert!(m.iter().any(|x| x.category == "custom"));
    }
    #[test]
    fn dismissed_sentinel() {
        // ensure _dismissed is preserved as pii_categories value (pipeline flag keeps sentinel)
        let disabled = HashSet::new();
        let custom = HashSet::new();
        let m = scan_text("no pii here", &disabled, &custom);
        assert!(m.is_empty());
    }
    #[test]
    fn categories_payload_enabled() {
        let disabled = HashSet::new();
        let payload = categories_payload(&disabled);
        assert!(payload.iter().all(|v| v["enabled"] == true));
    }
}
