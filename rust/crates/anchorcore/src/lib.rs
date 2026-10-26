//! AnchorCore library root — every module lives here so integration tests
//! (`tests/retrieval_eval.rs`) use the production code via `anchorcore::…`
//! instead of `#[path]`-compiling sources a second time (which ran 100 unit
//! tests twice). `main.rs` keeps only boot: CLI, logging, state, serve.

#![allow(dead_code)]
#![allow(clippy::unwrap_used)]

pub mod answer;
pub mod assets;
pub mod assets_api;
pub mod auth;
pub mod backup;
pub mod captions;
pub mod chunking;
pub mod classifier;
pub mod cli;
pub mod common;
pub mod config_chunks;
pub mod connectors;
pub mod db;
pub mod diagram;
pub mod diagram_formats;
pub mod embed_fp;
pub mod embed_local;
pub mod embed_model_api;
pub mod embedder;
pub mod entities;
pub mod extract;
pub mod fernet;
pub mod frontend;
pub mod fsbrowse;
pub mod glossary;
pub mod hashing;
pub mod health;
pub mod idf;
pub mod items;
pub mod jobs;
pub mod license;
pub mod llm_chat;
pub mod machine;
pub mod markdown;
pub mod mcp;
pub mod mcp_setup;
pub mod model_store;
pub mod ollama;
pub mod pdf_images;
pub mod pipeline;
pub mod projects;
pub mod query_expand;
pub mod records;
pub mod records_api;
pub mod report;
pub mod retrieval;
pub mod review;
pub mod routes;
pub mod runtime;
pub mod scheduler;
pub mod secrets;
pub mod sections;
pub mod security;
pub mod settings;
pub mod skills_setup;
pub mod sources;
pub mod stubs;
pub mod system;
pub mod tags;
pub mod tray;
pub mod vocab;
pub mod watcher;
