// src/models.rs
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use rmcp::schemars; // Required for JsonSchema derive

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MemoryItem {
    pub id: Uuid,
    pub text: String,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

impl MemoryItem {
    pub fn new(text: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            text,
            timestamp: Utc::now(),
            embedding: None,
        }
    }
}

// --- Tool Parameters and Responses ---

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddMemoryParams {
    pub memory: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AddMemoryResponse {
    pub id: String, // UUID as string
    pub status: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryMemoryParams {
    pub query_text: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}
fn default_top_k() -> usize { 5 }


#[derive(Serialize, Deserialize, Debug, Clone, schemars::JsonSchema)] // Added Deserialize and Clone for potential use
pub struct QueryResultItem {
    pub id: String, // UUID as string
    pub text: String,
    pub timestamp: DateTime<Utc>,
    pub score: f32,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct QueryMemoryResponse {
    pub results: Vec<String>,
}