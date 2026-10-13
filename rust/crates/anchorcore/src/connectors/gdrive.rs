use super::{ConnectorError, IngestionDoc};
use reqwest::Client;
use serde_json::Value;

const DRIVE_BASE: &str = "https://www.googleapis.com/drive/v3";
const LIST_FIELDS: &str = "nextPageToken, files(id, name, mimeType, modifiedTime, size, parents, trashed)";

pub struct GDriveConnector {
    folder_id: String,
    token: String,
    drive_id: String,
}

impl GDriveConnector {
    pub fn new(config: &serde_json::Value) -> Result<Self, ConnectorError> {
        let folder_id = config.get("folder_id").and_then(|v| v.as_str())
            .or_else(|| config.get("folder").and_then(|v| v.as_str()))
            .unwrap_or("").trim().to_string();
        let token = config.get("token").and_then(|v| v.as_str())
            .or_else(|| config.get("access_token").and_then(|v| v.as_str()))
            .unwrap_or("").trim().to_string();
        let drive_id = config.get("drive_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if folder_id.is_empty() { return Err(ConnectorError("gdrive connector requires folder_id".to_string())); }
        if token.is_empty() { return Err(ConnectorError("gdrive connector requires token".to_string())); }
        Ok(Self { folder_id, token, drive_id })
    }

    fn headers(&self) -> String { format!("Bearer {}", self.token) }

    async fn parse_file(&self, client: &Client, f: &Value, docs: &mut Vec<IngestionDoc>, skipped: &mut Vec<String>, next_cursor: &mut String) {
        let id = f.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let mime = f.get("mimeType").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let modified = f.get("modifiedTime").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let updated = super::parse_dt(&modified);
        let (kind, export_mime) = match plan_for(&mime, &name) {
            // subfolders are not ingested (flat listing for now — B28 polish)
            FilePlan::Folder => return,
            FilePlan::Unsupported(m) => {
                tracing::warn!("gdrive file skipped (unsupported type {}): {}", m, name);
                skipped.push(id);
                return;
            }
            FilePlan::Extract { kind, export_mime } => (kind, export_mime),
        };
        // Download/extract failures and empty texts are "skipped": the file
        // exists, so purge (B57) must never mistake it for deleted.
        let bytes = match self.download_bytes(client, &id, export_mime).await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("gdrive file skipped ({}): {}", name, e);
                skipped.push(id);
                return;
            }
        };
        let text = match crate::extract::extract_bytes_labeled(&kind, &bytes, &name) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("gdrive file skipped ({}): {}", name, e);
                skipped.push(id);
                return;
            }
        };
        if text.trim().is_empty() {
            skipped.push(id);
            return;
        }
        super::bump_cursor(next_cursor, updated);
        let source_ref = format!("gdrive:{}", id);
        docs.push(IngestionDoc { external_id: id, title: name, text, author: String::new(), updated_at: updated, source_ref });
    }

    /// B57/B28: like the folder connector, report files that failed to
    /// download/decode so a sync never hard-deletes them.
    pub async fn fetch_with_skipped(&self, since_cursor: &str) -> Result<(Vec<IngestionDoc>, Vec<String>, String), ConnectorError> {
        let client = super::http_client(60)?;
        let mut docs = Vec::new();
        let mut skipped = Vec::new();
        let mut next_cursor = since_cursor.to_string();
        let mut page_token: Option<String> = None;
        let mut q = format!("'{}' in parents and trashed = false", self.folder_id);
        if !since_cursor.is_empty() { q.push_str(&format!(" and modifiedTime > '{}'", since_cursor)); }
        let all_drives = if self.drive_id.is_empty() { "false" } else { "true" };

        loop {
            let mut params = vec![
                ("q", q.clone()),
                ("fields", LIST_FIELDS.to_string()),
                ("pageSize", "100".to_string()),
                ("orderBy", "modifiedTime asc".to_string()),
                ("includeItemsFromAllDrives", all_drives.to_string()),
                ("supportsAllDrives", all_drives.to_string()),
            ];
            if let Some(t) = &page_token { params.push(("pageToken", t.clone())); }
            let resp = client.get(format!("{}/files", DRIVE_BASE))
                .header("Authorization", self.headers())
                .query(&params)
                .send().await.map_err(|e| ConnectorError(e.to_string()))?;
            if resp.status().as_u16()==401 { return Err(ConnectorError("Drive auth failed (401) — token expired".to_string())); }
            if !resp.status().is_success() { return Err(ConnectorError(format!("Drive {}: {}", resp.status(), resp.text().await.unwrap_or_default()))); }
            let data: Value = resp.json().await.map_err(|e| ConnectorError(e.to_string()))?;
            for f in data.get("files").and_then(|v| v.as_array()).unwrap_or(&vec![]) {
                self.parse_file(&client, f, &mut docs, &mut skipped, &mut next_cursor).await;
            }
            page_token = data.get("nextPageToken").and_then(|v| v.as_str()).map(|s| s.to_string());
            if page_token.is_none() { break; }
        }
        Ok((docs, skipped, next_cursor))
    }

    pub async fn fetch(&self, since_cursor: &str) -> Result<(Vec<IngestionDoc>, String), ConnectorError> {
        let (docs, _skipped, cursor) = self.fetch_with_skipped(since_cursor).await?;
        Ok((docs, cursor))
    }

    async fn download_bytes(&self, client: &Client, file_id: &str, export_mime: Option<&str>) -> Result<Vec<u8>, ConnectorError> {
        let url = match export_mime {
            Some(m) => format!("{}/files/{}/export?mimeType={}", DRIVE_BASE, file_id, m),
            None => format!("{}/files/{}?alt=media", DRIVE_BASE, file_id),
        };
        let resp = client.get(&url).header("Authorization", self.headers()).send().await.map_err(|e| ConnectorError(e.to_string()))?;
        if !resp.status().is_success() { return Err(ConnectorError(format!("download {}: {}", resp.status(), resp.text().await.unwrap_or_default()))); }
        let bytes = resp.bytes().await.map_err(|e| ConnectorError(e.to_string()))?;
        Ok(bytes.to_vec())
    }
}

/// What to do with one Drive item: native Google types get an export format
/// that matches the shared extractors (`crate::extract`).
enum FilePlan {
    Folder,
    Unsupported(String),
    Extract { kind: String, export_mime: Option<&'static str> },
}

fn plan_for(mime: &str, name: &str) -> FilePlan {
    match mime {
        "application/vnd.google-apps.folder" => FilePlan::Folder,
        "application/vnd.google-apps.document" => FilePlan::Extract { kind: "txt".to_string(), export_mime: Some("text/plain") },
        "application/vnd.google-apps.spreadsheet" => FilePlan::Extract { kind: "csv".to_string(), export_mime: Some("text/csv") },
        "application/vnd.google-apps.presentation" => FilePlan::Extract { kind: "txt".to_string(), export_mime: Some("text/plain") },
        // drawings/forms/scripts and other native types export as PDF;
        // pdf-extract pulls the text out (was lossy mojibake)
        m if m.starts_with("application/vnd.google-apps.") => FilePlan::Extract { kind: "pdf".to_string(), export_mime: Some("application/pdf") },
        m => match crate::extract::kind_for(m, name) {
            Some(kind) => FilePlan::Extract { kind, export_mime: None },
            None => FilePlan::Unsupported(m.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_native_google_types() {
        assert!(matches!(plan_for("application/vnd.google-apps.folder", "x"), FilePlan::Folder));
        match plan_for("application/vnd.google-apps.document", "x") {
            FilePlan::Extract { kind, export_mime } => {
                assert_eq!(kind, "txt");
                assert_eq!(export_mime, Some("text/plain"));
            }
            _ => panic!("document should export as text"),
        }
        match plan_for("application/vnd.google-apps.spreadsheet", "x") {
            FilePlan::Extract { kind, export_mime } => {
                assert_eq!(kind, "csv");
                assert_eq!(export_mime, Some("text/csv"));
            }
            _ => panic!("spreadsheet should export as csv"),
        }
        match plan_for("application/vnd.google-apps.drawing", "x") {
            FilePlan::Extract { kind, export_mime } => {
                assert_eq!(kind, "pdf");
                assert_eq!(export_mime, Some("application/pdf"));
            }
            _ => panic!("drawing should export as pdf"),
        }
    }

    #[test]
    fn plan_uploaded_business_formats_extract_without_export() {
        for (mime, kind) in [
            ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "docx"),
            ("application/vnd.openxmlformats-officedocument.presentationml.presentation", "pptx"),
            ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "xlsx"),
            ("application/vnd.ms-excel", "xls"),
            ("application/pdf", "pdf"),
        ] {
            match plan_for(mime, "file") {
                FilePlan::Extract { kind: k, export_mime } => {
                    assert_eq!(k, kind, "{mime}");
                    assert_eq!(export_mime, None, "{mime}");
                }
                _ => panic!("{mime} should extract"),
            }
        }
    }

    #[test]
    fn plan_octet_stream_falls_back_to_filename() {
        match plan_for("application/octet-stream", "notes.docx") {
            FilePlan::Extract { kind, .. } => assert_eq!(kind, "docx"),
            _ => panic!("octet-stream + .docx should extract"),
        }
        assert!(matches!(plan_for("image/png", "logo.png"), FilePlan::Unsupported(_)));
    }
}
