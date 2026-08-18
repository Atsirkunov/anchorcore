"""PII knowledge base + detection (B30 partial).

A configurable catalog of *commonly recognised PII*: categories with the
attribute names / field names they usually travel under, plus regex patterns
for the values themselves, and user-supplied filter words. Used to:

- auto-flag chunks whose content matches PII during ingestion,
- power the PII configuration + review page,
- (later, on top of B39's trust gate) block sensitive content from cloud
  providers and non-public sharing.

Matching is local (regex + word scan) — no LLM, CI-safe, works offline.
"""

import json
import re
from dataclasses import dataclass, field
from typing import Any

from sqlalchemy.orm import Session

from .models import AppSetting

# config keys persisted in app_settings (B4-style non-secret overrides)
KEY_CUSTOM_WORDS = "pii_custom_words"
KEY_DISABLED_CATEGORIES = "pii_disabled_categories"


@dataclass
class PiiCategory:
    id: str
    label: str
    description: str
    field_names: tuple[str, ...] = ()
    patterns: tuple[str, ...] = ()
    strong: bool = True  # pattern hits count as strong (auto is_pii); name-only is weak


CATEGORIES: list[PiiCategory] = [
    PiiCategory(
        id="email",
        label="Email address",
        description="A personal email address / user account identifier.",
        field_names=("email", "email_address", "e-mail", "mail", "contact_email", "emailaddr"),
        patterns=(
            r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b",
        ),
    ),
    PiiCategory(
        id="phone",
        label="Phone number",
        description="Personal / mobile / work phone or fax number.",
        field_names=("phone", "phone_number", "mobile", "cell", "telephone", "tel", "fax", "contact_no", "work_phone"),
        patterns=(
            r"\b(?:\+?\d{1,3}[\s\-.]?)?(?:\(\d{1,4}\)[\s\-.]?)?\d{3}[\s\-.]?\d{3}[\s\-.]?\d{4}\b",
        ),
    ),
    PiiCategory(
        id="full_name",
        label="Full name",
        description="A person's given / family / full name.",
        field_names=("full_name", "first_name", "last_name", "given_name", "family_name", "name_of", "display_name", "legal_name"),
    ),
    PiiCategory(
        id="ssn",
        label="Social Security Number",
        description="US SSN (###-##-####).",
        field_names=("ssn", "social_security", "social_security_number", "ssn_number", "socialsecurity"),
        patterns=(r"\b\d{3}[\-.\s]?\d{2}[\-.\s]?\d{4}\b",),
    ),
    PiiCategory(
        id="passport",
        label="Passport number",
        description="Passport / travel document number.",
        field_names=("passport", "passport_number", "passport_no", "passportno"),
        patterns=(r"\b[A-Z]{1,2}\d{6,9}\b",),
    ),
    PiiCategory(
        id="drivers_license",
        label="Driver's license",
        description="Driver's license / state ID number.",
        field_names=("drivers_license", "driver_license", "driver_license_number", "license_number", "dl_number", "lic_no"),
        patterns=(r"\b(?:DL|LIC)[\s\-:]*\d{6,9}\b",),
    ),
    PiiCategory(
        id="national_id",
        label="National ID number",
        description="Government-issued ID: NIN, CPF, Aadhaar, citizen ID, etc.",
        field_names=("national_id", "national_identification", "id_number", "citizen_id", "personal_id", "nin", "cpf", "aadhaar", "identity_no", "ic_number"),
        patterns=(
            r"\b\d{3}\.?\d{3}\.?\d{3}[\-]?\d{2}\b",  # CPF 000.000.000-00
            r"\b\d{12}\b",  # Aadhaar
        ),
    ),
    PiiCategory(
        id="dob",
        label="Date of birth",
        description="Date of birth.",
        field_names=("dob", "date_of_birth", "birth_date", "birthday", "birthdate", "birth_date_of"),
        patterns=(r"\b(?:born|dob|date of birth)[:\s]+(\d{1,2}[/\-.]\d{1,2}[/\-.]\d{2,4})\b",),
    ),
    PiiCategory(
        id="address",
        label="Home address",
        description="Street address, city, state, postal code.",
        field_names=("address", "street", "street_address", "home_address", "mailing_address", "city", "postal", "postal_code", "zip", "zip_code", "state", "province"),
        patterns=(
            r"\b\d{1,5}\s+[A-Za-z0-9 .\-,]+(?:Street|St|Avenue|Ave|Road|Rd|Boulevard|Blvd|Lane|Ln|Drive|Dr|Way|Court|Ct|Place|Pl)\b",
            r"\b[A-Z]{1,2}\d[A-Z\d]?\s?\d[A-Z]{2}\b",  # UK postcode
        ),
    ),
    PiiCategory(
        id="ip_address",
        label="IP address",
        description="IPv4 / IPv6 address that can identify a device.",
        field_names=("ip", "ip_address", "client_ip", "remote_addr", "ip_addr", "source_ip"),
        patterns=(
            r"\b(?:\d{1,3}\.){3}\d{1,3}\b",
            r"\b[0-9a-fA-F]{1,4}:(?:[0-9a-fA-F]{0,4}:){2,7}[0-9a-fA-F]{1,4}\b",
        ),
    ),
    PiiCategory(
        id="credit_card",
        label="Credit card number",
        description="Payment card PAN (Visa/MC/Amex).",
        field_names=("credit_card", "card_number", "cc_number", "cc", "pan", "cardholder", "card_no"),
        patterns=(
            r"\b(?:\d{4}[\s\-]?){3}\d{4}\b",
            r"\b\d{4}[\s\-]?\d{4}[\s\-]?\d{4}[\s\-]?\d{4}\b",
        ),
    ),
    PiiCategory(
        id="bank_account",
        label="Bank account number",
        description="Bank / savings / routing account number.",
        field_names=("bank_account", "account_number", "bank_acct", "routing_number", "sort_code", "acct_no", "bank_account_no"),
        patterns=(r"\b\d{8,17}\b",),
    ),
    PiiCategory(
        id="iban",
        label="IBAN",
        description="International Bank Account Number.",
        field_names=("iban", "iban_number", "swift", "bic"),
        patterns=(r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b",),
    ),
    PiiCategory(
        id="tax_id",
        label="Tax ID (TIN/EIN)",
        description="Taxpayer / employer identification number.",
        field_names=("tax_id", "tin", "ein", "employer_id", "vat", "vat_number", "tax_identification"),
        patterns=(r"\b\d{2}[\-.]?\d{7}\b",),
    ),
    PiiCategory(
        id="credentials",
        label="Credentials / API keys",
        description="Passwords, API keys, tokens, client secrets.",
        field_names=("password", "passwd", "api_key", "apikey", "secret", "secret_key", "token", "access_token", "refresh_token", "client_secret", "auth", "authorization", "bearer"),
        patterns=(
            r"\b(?:sk|pk|ghp|gho|xox[baprs]|AKIA)[A-Za-z0-9_\-]{16,}\b",
            r"\bpassword\s*[=:]\s*\S+",
            r"\bapi[_-]?key\s*[=:]\s*\S+",
        ),
    ),
    PiiCategory(
        id="health",
        label="Medical / health record",
        description="Medical record number, diagnosis, condition, blood type.",
        field_names=("mrn", "medical_record", "medical_record_number", "patient_id", "diagnosis", "medical_condition", "blood_type", "patient_no", "health_record"),
        patterns=(r"\bMRN[\s\-:]*\d{4,}\b",),
    ),
    PiiCategory(
        id="employment",
        label="Employee details",
        description="Employee ID, salary, compensation, job details.",
        field_names=("employee_id", "emp_id", "employee_no", "salary", "compensation", "pay_rate", "hire_date", "manager_id"),
    ),
    PiiCategory(
        id="biometric",
        label="Biometric data",
        description="Fingerprint, retina/iris scan, face ID, voice print.",
        field_names=("fingerprint", "retina", "iris", "face_id", "faceid", "voice_print", "biometric", "biometric_id"),
        patterns=(r"\b(?:fingerprint|retina scan|iris scan|face id|voiceprint)\b",),
        strong=False,
    ),
    PiiCategory(
        id="demographics",
        label="Demographics (protected class)",
        description="Ethnicity, religion, marital status, gender, nationality.",
        field_names=("ethnicity", "race", "religion", "marital_status", "civil_status", "gender", "sex", "nationality", "sexual_orientation"),
    ),
    PiiCategory(
        id="age",
        label="Age",
        description="Age or age range.",
        field_names=("age", "age_group", "age_range"),
        patterns=(r"\bage\s*[=:]\s*\d{1,3}\b",),
        strong=False,
    ),
]

# canonical map for lookups
_CATEGORY_BY_ID = {c.id: c for c in CATEGORIES}


@dataclass
class PiiMatch:
    category: str
    label: str
    strong: bool
    match: str = ""


def _load_json_set(db: Session, key: str) -> set[str]:
    row = db.get(AppSetting, key)
    if row is None or not row.value:
        return set()
    try:
        return set(json.loads(row.value))
    except (json.JSONDecodeError, TypeError):
        return set()


def load_config(db: Session) -> dict[str, Any]:
    """Effective PII config: enabled categories + user filter words."""
    disabled = _load_json_set(db, KEY_DISABLED_CATEGORIES)
    custom = _load_json_set(db, KEY_CUSTOM_WORDS)
    return {"disabled": disabled, "custom_words": custom}


def save_config(db: Session, *, disabled: list[str] | None = None, custom_words: list[str] | None = None) -> None:
    if disabled is not None:
        row = db.get(AppSetting, KEY_DISABLED_CATEGORIES)
        if row is None:
            db.add(AppSetting(key=KEY_DISABLED_CATEGORIES, value=json.dumps(sorted(set(disabled)))))
        else:
            row.value = json.dumps(sorted(set(disabled)))
    if custom_words is not None:
        row = db.get(AppSetting, KEY_CUSTOM_WORDS)
        if row is None:
            db.add(AppSetting(key=KEY_CUSTOM_WORDS, value=json.dumps(sorted(set(custom_words)))))
        else:
            row.value = json.dumps(sorted(set(custom_words)))
    db.commit()


def scan_text(text: str, *, disabled: set[str] | None = None, custom_words: set[str] | None = None) -> list[PiiMatch]:
    """Scan one text blob for PII. Returns matches (category id, label,
    strong, matched snippet). Local-only, no LLM.

    A category hits when one of its value *patterns* matches the text (strong
    signal) OR one of its field names appears as a word (weak — the field is
    named, but we don't validate the value). User filter words are strong."""
    disabled = disabled or set()
    custom_words = custom_words or set()
    matches: list[PiiMatch] = []
    lower = text.lower()

    for cat in CATEGORIES:
        if cat.id in disabled:
            continue
        hit = False
        strong = False
        snippet = ""
        for pattern in cat.patterns:
            m = re.search(pattern, text, re.IGNORECASE)
            if m:
                hit = True
                strong = cat.strong
                snippet = m.group(0)
                break
        if not hit:
            for name in cat.field_names:
                if re.search(rf"\b{re.escape(name)}\b", lower):
                    hit = True
                    strong = False  # field named, value unvalidated
                    snippet = name
                    break
        if hit:
            matches.append(PiiMatch(category=cat.id, label=cat.label, strong=strong, match=snippet))

    for word in custom_words:
        w = word.strip()
        if not w:
            continue
        if re.search(rf"\b{re.escape(w.lower())}\b", lower):
            matches.append(PiiMatch(category="custom", label=w, strong=True, match=w))

    # de-dupe by category (first hit wins, strong preferred)
    seen: dict[str, PiiMatch] = {}
    for m in matches:
        if m.category not in seen or (m.strong and not seen[m.category].strong):
            seen[m.category] = m
    return list(seen.values())


def categories_payload(disabled: set[str]) -> list[dict[str, Any]]:
    """Serialisable category catalog for the config page."""
    return [
        {
            "id": c.id,
            "label": c.label,
            "description": c.description,
            "field_names": list(c.field_names),
            "patterns": list(c.patterns),
            "enabled": c.id not in disabled,
        }
        for c in CATEGORIES
    ]


def serialize_matches(matches: list[PiiMatch]) -> list[dict[str, Any]]:
    return [
        {
            "category": m.category,
            "label": m.label,
            "strong": m.strong,
            "match": m.match,
        }
        for m in matches
    ]


__all__ = [
    "CATEGORIES",
    "PiiCategory",
    "PiiMatch",
    "categories_payload",
    "load_config",
    "save_config",
    "scan_text",
    "serialize_matches",
]
