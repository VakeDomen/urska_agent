// src/main.rs
mod config;
mod models;
mod ollama_service;
mod qdrant_service;
mod mcp_service;

use std::sync::Arc;
use crate::config::Config;
use crate::ollama_service::OllamaService;
use crate::qdrant_service::QdrantService;
use crate::mcp_service::{MemoryMcpService, AppState};
use anyhow::Result;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::StreamableHttpService;

const BIND_ADDRESS: &str = "127.0.0.1:8002";

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger (optional, but good practice)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // 1. Load Configuration
    let app_config = Config::from_env().expect("Failed to load configuration from .env file");
    println!("Configuration loaded. Bind address: {}", app_config.bind_address);

    // 2. Initialize Services (Ollama, Qdrant)
    let ollama_serv = OllamaService::new(&app_config);
    println!("Ollama service configured for endpoint: {}", app_config.ollama_endpoint);

    let qdrant_serv = QdrantService::new(&app_config).await
        .expect("Failed to initialize Qdrant service");
    println!("Qdrant service configured for endpoint: {} and collection: {}", app_config.qdrant_endpoint, app_config.qdrant_collection_name);

    // 3. Create AppState
    let app_state = Arc::new(AppState::new(ollama_serv, qdrant_serv));
    println!("Application state created.");

    // 4. Serve the service using rmcp's axum_server
    // This will start an HTTP server that handles RMCP requests for the tools.

    let service = StreamableHttpService::new(
        move || Ok(MemoryMcpService::new(app_state.clone())),
        LocalSessionManager::default().into(),
        Default::default(),
    );

    let router = axum::Router::new().nest_service("/mcp", service);
    let tcp_listener = tokio::net::TcpListener::bind(BIND_ADDRESS).await?;
    let _ = axum::serve(tcp_listener, router)
        .with_graceful_shutdown(async { tokio::signal::ctrl_c().await.unwrap() })
        .await;

    // let ct = sse_server
    //     .with_service(move || MemoryMcpService::new(app_state.clone()));
    // let ct = sse_server.with_service(move || Counter::new());

    tokio::signal::ctrl_c().await?;
    // ct.cancel();
    Ok(())
}