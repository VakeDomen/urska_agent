// src/config.rs
use dotenv::dotenv;
use std::env;

#[derive(Debug, Clone)] // Added Clone
pub struct Config {
    pub ollama_endpoint: String,
    pub embedding_model: String,
    pub qdrant_endpoint: String,
    pub qdrant_collection_name: String,
    pub embedding_dimension: u64,
    pub bind_address: String,
}

impl Config {
    pub fn from_env() -> Result<Self, anyhow::Error> {
        dotenv().ok();
        Ok(Self {
            ollama_endpoint: env::var("OLLAMA_ENDPOINT").unwrap_or_else(|_| "http://localhost:11434".to_string()),
            embedding_model: env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "bge-m3".to_string()),
            qdrant_endpoint: env::var("QDRANT_ENDPOINT").unwrap_or_else(|_| "http://localhost:6334".to_string()),
            qdrant_collection_name: env::var("QDRANT_COLLECTION_NAME").unwrap_or_else(|_| "agent_memory_rmcp".to_string()),
            embedding_dimension: env::var("EMBEDDING_DIMENSION")
                .unwrap_or_else(|_| "1024".to_string())
                .parse::<u64>()?,
            bind_address: env::var("BIND_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8002".to_string()),
        })
    }
}