// src/ollama_service.rs
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use crate::config::Config; // Assuming config.rs is in the same crate root

#[derive(Serialize)]
struct OllamaEmbeddingRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    // stream: bool, // According to ollama-rs, 'stream' is not part of embedding req
}

#[derive(Deserialize, Debug)] // Added Debug
struct OllamaEmbeddingResponse {
    embedding: Vec<f32>,
}

#[derive(Debug, Clone)] // Added Clone for easier use in AppState
pub struct OllamaService {
    client: HttpClient,
    ollama_endpoint: String,
    embedding_model: String,
}

impl OllamaService {
    pub fn new(config: &Config) -> Self {
        Self {
            client: HttpClient::new(),
            ollama_endpoint: config.ollama_endpoint.clone(),
            embedding_model: config.embedding_model.clone(),
        }
    }

    pub async fn get_embedding(&self, text: &str) -> Result<Vec<f32>, anyhow::Error> {
        let request_url = format!("{}/api/embeddings", self.ollama_endpoint);
        // println!("Requesting embedding from: {} for model: {}", request_url, self.embedding_model);
        let response = self.client
            .post(&request_url)
            .json(&OllamaEmbeddingRequest {
                model: &self.embedding_model,
                prompt: text,
            })
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Ollama request failed: {}", e))?
            .json::<OllamaEmbeddingResponse>()
            .await
            .map_err(|e| anyhow::anyhow!("Ollama response deserialization failed: {}", e))?;
        Ok(response.embedding)
    }
}