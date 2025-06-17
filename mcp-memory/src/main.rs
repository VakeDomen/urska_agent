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
use rmcp::transport::sse_server::SseServerConfig;
use rmcp::transport::SseServer;

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

    let config = SseServerConfig {
        bind: app_config.bind_address.parse()?,
        sse_path: "/sse".to_string(),
        post_path: "/message".to_string(),
        ct: tokio_util::sync::CancellationToken::new(),
        sse_keep_alive: None,
    };
    println!("Starting RMCP server on {}...", app_config.bind_address);
    let (sse_server, router) = SseServer::new(config);

    // Do something with the router, e.g., add routes or middleware

    let listener = tokio::net::TcpListener::bind(sse_server.config.bind).await?;

    let ct = sse_server.config.ct.child_token();

    let server = axum::serve(listener, router).with_graceful_shutdown(async move {
        ct.cancelled().await;
        println!("sse server cancelled");
    });

    tokio::spawn(async move {
        if let Err(e) = server.await {
            println!("sse server shutdown with error: {}", e.to_string());
        }
    });

    let ct = sse_server
        .with_service(move || MemoryMcpService::new(app_state.clone()));
    // let ct = sse_server.with_service(move || Counter::new());

    tokio::signal::ctrl_c().await?;
    ct.cancel();
    Ok(())
}