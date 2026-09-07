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

    async fn parse_file(&self, client: &Client, f: &Value, docs: &mut Vec<IngestionDoc>, next_cursor: &mut String) {
        let id = f.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let mime = f.get("mimeType").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let modified = f.get("modifiedTime").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let updated = super::parse_dt(&modified);
        // download content (text export)
        let text = self.download_file(client, &id, &mime, &name).await.unwrap_or_default();
        if text.trim().is_empty() {
            return;
        }
        // empty text already filtered above, so push directly (keeps gdrive: prefix shape)
        super::bump_cursor(next_cursor, updated);
        docs.push(IngestionDoc { external_id: id.clone(), title: name.clone(), text, author: String::new(), updated_at: updated, source_ref: format!("gdrive:{}", id) });
    }

    pub async fn fetch(&self, since_cursor: &str) -> Result<(Vec<IngestionDoc>, String), ConnectorError> {
        let client = super::http_client(60)?;
        let mut docs = Vec::new();
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
                self.parse_file(&client, f, &mut docs, &mut next_cursor).await;
            }
            page_token = data.get("nextPageToken").and_then(|v| v.as_str()).map(|s| s.to_string());
            if page_token.is_none() { break; }
        }
        Ok((docs, next_cursor))
    }

    async fn download_file(&self, client: &Client, file_id: &str, mime: &str, _name: &str) -> Result<String, ConnectorError> {
        // native Google docs -> export
        let url = if mime.starts_with("application/vnd.google-apps.") {
            let export_mime = match mime {
                "application/vnd.google-apps.document" => "text/plain",
                "application/vnd.google-apps.presentation" => "text/plain",
                "application/vnd.google-apps.spreadsheet" => "text/csv",
                _ => "application/pdf",
            };
            format!("{}/files/{}/export?mimeType={}", DRIVE_BASE, file_id, export_mime)
        } else {
            format!("{}/files/{}?alt=media", DRIVE_BASE, file_id)
        };
        let resp = client.get(&url).header("Authorization", self.headers()).send().await.map_err(|e| ConnectorError(e.to_string()))?;
        if !resp.status().is_success() { return Err(ConnectorError(format!("download {}: {}", resp.status(), resp.text().await.unwrap_or_default()))); }
        let bytes = resp.bytes().await.map_err(|e| ConnectorError(e.to_string()))?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }
}
