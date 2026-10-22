//! Black-box API contract harness: spawns the real `anchorcore` binary on a
//! free port with a fresh temp data dir, then drives it over HTTP.
//!
//! This replaces the archived Python conformance suite (retired — the frozen
//! `backend/tests` asserted deleted behaviour and tested Python internals
//! against themselves). One server per test: fresh DB, no shared state, no
//! serial lock. Env mirrors CI's `rust-conformance` leg.

//! Each test target compiles this module separately and uses a different
//! helper subset; unused helpers in any one target are not dead code.
#![allow(dead_code)]

use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("ephemeral addr")
        .port()
}

/// Proven record shape (minimal `Decision:` doc — yields one decision entity
/// under the rules classifier). Keep fixtures unique per test via `topic`.
pub fn decision_doc(title: &str, topic: &str, owner: &str) -> String {
    format!(
        "# Decision: {title}\n\n**Decision:** {topic}.\n\n**Status:** Approved. Owner: {owner}.\n"
    )
}

pub fn action_doc(topic: &str, owner: &str) -> String {
    // Mirrors the proven retro shape: `Attendees:` buckets the doc as a
    // record (an action-list-only doc parses nothing — R45.14), and the
    // Owner/Due continuation line is what the parser reads.
    format!(
        "# Sprint notes\n\nAttendees: {owner}, Sam.\n\n## Action items\n\n- {owner}: {topic}.\n  Owner: {owner}. Due: end of this week.\n"
    )
}

pub struct TestServer {
    child: Child, // declared first: killed before the temp dirs are removed
    pub port: u16,
    pub dir: TempDir,
    pub work: TempDir,
    pub client: Client,
    csrf: String,
}

impl TestServer {
    pub async fn start() -> Self {
        Self::start_with_env(&[]).await
    }

    /// `extra_env` overrides the deterministic defaults (e.g.
    /// `ANCHOR_CLOUD_TRUST=1`, `ANCHOR_QA_EXCLUDE_DISPUTED=0`).
    pub async fn start_with_env(extra_env: &[(&str, &str)]) -> Self {
        let port = free_port();
        let dir = TempDir::new().expect("data tempdir");
        let work = TempDir::new().expect("work tempdir");
        let mut srv = Self {
            child: Self::spawn(port, dir.path(), extra_env),
            port,
            dir,
            work,
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
            csrf: String::new(),
        };
        srv.wait_healthy().await;
        let token = srv.get_json("/csrf").await;
        srv.csrf = token["csrf_token"]
            .as_str()
            .expect("csrf_token in /csrf")
            .to_string();
        srv
    }

    fn spawn(port: u16, data_dir: &std::path::Path, extra_env: &[(&str, &str)]) -> Child {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_anchorcore"));
        cmd.arg("--port")
            .arg(port.to_string())
            .arg("--data-dir")
            .arg(data_dir)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--no-tray")
            // Deterministic defaults: never inherit the dev machine's DB,
            // trust flags, or keychain. Mirrors CI's conformance env.
            .env_remove("ANCHOR_DATA_DIR")
            .env_remove("ANCHOR_DATABASE_URL")
            .env_remove("ANCHOR_CLOUD_TRUST")
            .env_remove("ANCHOR_TRUSTED_PROVIDERS")
            .env_remove("ANCHOR_QA_EXCLUDE_DISPUTED")
            .env("ANCHOR_SECRETS_NO_KEYRING", "1")
            .env("ANCHOR_OLLAMA_BASE_URL", "http://localhost:1")
            .env("ANCHOR_ALLOW_TEST_URLS", "1")
            .env("ANCHOR_CLASSIFIER_TIMEOUT", "1.0")
            .env("ANCHOR_HTTP_CONNECT_TIMEOUT", "0.2")
            .env("ANCHOR_ANSWER_BASE_URL", "https://api.openai.com/v1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd.spawn().expect("spawn anchorcore binary")
    }

    /// Stop and start a new process on the same port + data dir (startup
    /// sweep, fresh-env reads). The old process is reaped first.
    pub async fn restart(&mut self, extra_env: &[(&str, &str)]) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.child = Self::spawn(self.port, self.dir.path(), extra_env);
        self.wait_healthy().await;
        let token = self.get_json("/csrf").await;
        self.csrf = token["csrf_token"]
            .as_str()
            .expect("csrf_token in /csrf")
            .to_string();
    }

    async fn wait_healthy(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let url = self.url("/health");
        while Instant::now() < deadline {
            if let Ok(resp) = self.client.get(&url).send().await {
                if resp.status() == StatusCode::OK {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let log_path = self.dir.path().join("anchorcore.log");
        let tail = std::fs::read_to_string(&log_path)
            .map(|s| {
                let mut lines: Vec<&str> = s.lines().collect();
                lines.split_off(lines.len().saturating_sub(20)).join("\n")
            })
            .unwrap_or_else(|e| format!("<no log: {e}>"));
        panic!("server on :{} never healthy; log tail:\n{tail}", self.port);
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn db_path(&self) -> std::path::PathBuf {
        self.dir.path().join("anchorcore.db")
    }

    /// Direct DB access for white-box asserts (cascade deletes, planted rows).
    pub fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.db_path()).expect("open server db")
    }

    pub async fn get(&self, path: &str) -> Response {
        self.client
            .get(self.url(path))
            .send()
            .await
            .expect("GET request")
    }

    pub async fn get_json(&self, path: &str) -> Value {
        let resp = self.get(path).await;
        assert_eq!(resp.status(), StatusCode::OK, "GET {path}");
        resp.json().await.expect("json body")
    }

    async fn mutating(&self, req: reqwest::RequestBuilder) -> Response {
        req.header("X-CSRF-Token", &self.csrf)
            .send()
            .await
            .expect("mutating request")
    }

    pub async fn post(&self, path: &str, body: &Value) -> Response {
        let req = self.client.post(self.url(path)).json(body);
        self.mutating(req).await
    }

    pub async fn post_empty(&self, path: &str) -> Response {
        let req = self.client.post(self.url(path));
        self.mutating(req).await
    }

    pub async fn put(&self, path: &str, body: &Value) -> Response {
        let req = self.client.put(self.url(path)).json(body);
        self.mutating(req).await
    }

    pub async fn patch(&self, path: &str, body: &Value) -> Response {
        let req = self.client.patch(self.url(path)).json(body);
        self.mutating(req).await
    }

    pub async fn delete(&self, path: &str) -> Response {
        let req = self.client.delete(self.url(path));
        self.mutating(req).await
    }

    /// Create a folder source over freshly written files. `label` sets the
    /// source label (`internal` default). Asserts 201.
    pub async fn mk_folder_source(
        &self,
        name: &str,
        files: &[(&str, &str)],
        label: Option<&str>,
    ) -> Value {
        let root = self.work.path().join(name);
        std::fs::create_dir_all(&root).expect("corpus dir");
        for (rel, content) in files {
            std::fs::write(root.join(rel), content).expect("corpus file");
        }
        let mut body = json!({
            "connector": "folder",
            "name": name,
            "config": {"path": root.to_string_lossy()},
        });
        if let Some(label) = label {
            body["label"] = json!(label);
        }
        let resp = self.post("/sources", &body).await;
        assert_eq!(resp.status(), StatusCode::CREATED, "POST /sources {name}");
        resp.json().await.expect("source json")
    }

    /// POST a sync/reclassify job and poll to a terminal state. Asserts 202.
    pub async fn sync_and_wait(&self, source_id: i64, kind: &str) -> Value {
        let resp = self
            .post_empty(&format!("/sources/{source_id}/{kind}"))
            .await;
        assert_eq!(
            resp.status(),
            StatusCode::ACCEPTED,
            "POST /sources/{source_id}/{kind}"
        );
        let job: Value = resp.json().await.expect("job json");
        self.wait_job(job["id"].as_i64().expect("job id")).await
    }

    pub async fn wait_job(&self, job_id: i64) -> Value {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let resp = self.get(&format!("/sources/jobs/{job_id}")).await;
            assert_eq!(resp.status(), StatusCode::OK, "GET /sources/jobs/{job_id}");
            let job: Value = resp.json().await.expect("job json");
            if job["status"] != "running" && job["status"] != "pending" {
                return job;
            }
            assert!(
                Instant::now() < deadline,
                "job {job_id} stuck in {:?}",
                job["status"]
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
