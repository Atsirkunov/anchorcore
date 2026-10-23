use super::{ConnectorError, IngestionDoc};
use reqwest::Client;
use serde_json::Value;

const DEFAULT_BASE: &str = "https://www.googleapis.com/drive/v3";
const LIST_FIELDS: &str = "nextPageToken, files(id, name, mimeType, modifiedTime, size, parents, trashed)";

pub struct GDriveConnector {
    folder_id: String,
    token: String,
    drive_id: String,
    /// Drive API root (`base_url` config; tests point it at a mock).
    base: String,
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
        let base = config.get("base_url").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_BASE).trim_end_matches('/').to_string();
        Ok(Self { folder_id, token, drive_id, base })
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
        // R46.15: no file on disk to re-read, so Office images resolve from
        // the bytes already downloaded (never a second download).
        let package = text.contains(crate::assets::EMBEDDED_SCHEME).then(|| std::sync::Arc::new(bytes));
        docs.push(IngestionDoc { external_id: id, title: name, text, author: String::new(), updated_at: updated, source_ref, package });
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
            let resp = client.get(format!("{}/files", self.base))
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
            Some(m) => format!("{}/files/{}/export?mimeType={}", self.base, file_id, m),
            None => format!("{}/files/{}?alt=media", self.base, file_id),
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
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Drive API mock: one folder listing (no paging) + `alt=media`
    /// downloads of `(id, name, mimeType, bytes)` files. Every request
    /// target lands in `hits`.
    pub(crate) struct MockDrive {
        pub base: String,
        pub hits: Arc<Mutex<Vec<String>>>,
        _task: tokio::task::JoinHandle<()>,
    }

    impl MockDrive {
        pub fn hits_for(&self, needle: &str) -> usize {
            self.hits.lock().unwrap().iter().filter(|h| h.contains(needle)).count()
        }
    }

    pub(crate) async fn spawn_mock(files: Vec<(&'static str, &'static str, &'static str, Vec<u8>)>) -> MockDrive {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let h = hits.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else { break };
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
                h.lock().unwrap().push(target.clone());
                let listing = serde_json::json!({"files": files.iter().map(|(id, name, mime, _)| serde_json::json!({
                    "id": id, "name": name, "mimeType": mime, "modifiedTime": "2026-10-20T10:00:00Z"
                })).collect::<Vec<_>>()});
                let (status, body) = match target.strip_prefix("/files/").and_then(|r| r.split_once("?alt=media")) {
                    Some((id, _)) => match files.iter().find(|f| f.0 == id) {
                        Some(f) => (200, f.3.clone()),
                        None => (404, b"{}".to_vec()),
                    },
                    None if target.starts_with("/files?") => (200, listing.to_string().into_bytes()),
                    None => (404, b"{}".to_vec()),
                };
                let reason = if status == 200 { "OK" } else { "Not Found" };
                let resp = format!("HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(resp.as_bytes()).await;
                let _ = s.write_all(&body).await;
            }
        });
        MockDrive { base: format!("http://{addr}"), hits, _task: task }
    }

    #[tokio::test]
    async fn office_package_rides_along_only_with_image_markers() {
        // R46.15: the downloaded docx stays on the doc when its text has
        // `embedded:` markers; a plain doc carries nothing.
        use std::io::Write;
        let docx = |body: &str, media: bool| {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("word/document.xml", opts).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
            if media {
                zip.start_file("word/_rels/document.xml.rels", opts).unwrap();
                zip.write_all(br#"<Relationships><Relationship Id="r1" Target="media/a.png"/></Relationships>"#).unwrap();
            }
            zip.finish().unwrap().into_inner()
        };
        let pic = r#"<w:document xmlns:w="w" xmlns:wp="wp" xmlns:a="a" xmlns:r="r"><w:body><w:p><w:r><w:t>Flow.</w:t></w:r></w:p><w:p><w:r><w:drawing><wp:docPr id="1" descr="Map"/><a:blip r:embed="r1"/></w:drawing></w:r></w:p></w:body></w:document>"#;
        let plain = r#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>Just text.</w:t></w:r></w:p></w:body></w:document>"#;
        let mime = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
        let mock = spawn_mock(vec![("d1", "pic.docx", mime, docx(pic, true)), ("d2", "plain.docx", mime, docx(plain, false))]).await;
        let gc = GDriveConnector::new(&serde_json::json!({"folder_id": "f", "token": "t", "base_url": mock.base})).unwrap();
        let (docs, skipped, _) = gc.fetch_with_skipped("").await.unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        let by_id = |id: &str| docs.iter().find(|d| d.external_id == id).unwrap();
        assert!(by_id("d1").text.contains("](embedded:word/media/a.png)"), "{}", by_id("d1").text);
        assert!(by_id("d1").package.as_ref().is_some_and(|p| p.starts_with(b"PK")));
        assert!(by_id("d2").package.is_none());
        assert_eq!(mock.hits_for("alt=media"), 2, "one download per file");
    }

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
