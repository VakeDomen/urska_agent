// src/memory_service.rs
use rmcp::{
    Request, Service, ServiceInfo, ServiceInfoBuilder, StreamItem,
    StreamExt, // For request_stream.next().await
};
use tokio::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use crate::models::{
    MemoryItem, AddMemoryCommand, QueryMemoryCommand,
    MemoryAddedResponse, QueryResultItem, QueryMemoryResponse
};
use crate::ollama_service::OllamaService;
use crate::qdrant_service::QdrantService;
use qdrant_client::qdrant::ScoredPoint; // Import ScoredPoint
use serde_json::Value as JsonValue; // Alias for clarity

const STM_CAPACITY: usize = 20; // Max items in Short-Term Memory

#[derive(Clone)] // AppState needs to be Clone
pub struct AppState {
    // Short-Term Memory: In-memory buffer
    stm: Arc<Mutex<VecDeque<MemoryItem>>>,
    ollama_service: Arc<OllamaService>,
    qdrant_service: Arc<QdrantService>,
}

impl AppState {
    pub fn new(ollama_service: OllamaService, qdrant_service: QdrantService) -> Self {
        Self {
            stm: Arc::new(Mutex::new(VecDeque::with_capacity(STM_CAPACITY))),
            ollama_service: Arc::new(ollama_service),
            qdrant_service: Arc::new(qdrant_service),
        }
    }

    // Helper to add to STM
    fn add_to_stm(&self, item: MemoryItem) {
        let mut stm_guard = self.stm.lock().unwrap();
        if stm_guard.len() >= STM_CAPACITY {
            stm_guard.pop_front(); // Remove oldest if capacity reached
        }
        stm_guard.push_back(item);
    }
}

pub struct MemoryService {
    state: AppState,
}

impl MemoryService {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait::async_trait]
impl Service for MemoryService {
    fn info(&self) -> ServiceInfo {
        ServiceInfoBuilder::new("memory.mcp", "Agent Memory Service")
            .method("add_memory", "Adds a new memory item.")
            .method("query_memory", "Queries existing memories.")
            .build()
    }

    async fn handle(
        &self,
        request: Request,
        tx: mpsc::Sender<StreamItem>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        println!("Handling request: {}", request.method);

        match request.method.as_str() {
            "add_memory" => {
                let command: AddMemoryCommand = serde_json::from_value(request.payload)?;
                let mut memory_item = MemoryItem::new(command.text.clone(), command.metadata);

                // 1. Add to STM (without embedding initially, or embed if needed for STM tasks)
                // For this example, STM just stores the raw text and metadata.
                self.state.add_to_stm(memory_item.clone());
                println!("Added to STM: {}", memory_item.id);

                // 2. Get embedding from Ollama
                match self.state.ollama_service.get_embedding(&command.text).await {
                    Ok(embedding) => {
                        memory_item.embedding = Some(embedding);

                        // 3. Store in Qdrant (LTM)
                        if let Err(e) = self.state.qdrant_service.add_memory(&memory_item).await {
                            eprintln!("Error adding memory to Qdrant: {}", e);
                            let error_item = StreamItem::error(request.id, 500, format!("Qdrant error: {}", e));
                            tx.send(error_item).await?;
                            return Ok(());
                        }
                        println!("Added to LTM (Qdrant): {}", memory_item.id);

                        let response = MemoryAddedResponse {
                            id: memory_item.id,
                            status: "Memory added to STM and LTM".to_string(),
                            timestamp: memory_item.timestamp,
                        };
                        tx.send(StreamItem::ok(request.id, serde_json::to_value(response)?)).await?;
                    }
                    Err(e) => {
                        eprintln!("Error getting embedding from Ollama: {}", e);
                         let error_item = StreamItem::error(request.id, 500, format!("Ollama embedding error: {}", e));
                        tx.send(error_item).await?;
                    }
                }
            }
            "query_memory" => {
                let command: QueryMemoryCommand = serde_json::from_value(request.payload)?;
                let top_k = command.top_k.unwrap_or(5);

                match self.state.ollama_service.get_embedding(&command.query_text).await {
                    Ok(query_embedding) => {
                        match self.state.qdrant_service.search_memories(query_embedding, top_k).await {
                            Ok(search_results) => {
                                let mut results: Vec<QueryResultItem> = Vec::new();
                                for point in search_results {
                                    let payload = point.payload;
                                    // Ensure payload fields are correctly extracted
                                    let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                                    let timestamp_str = payload.get("timestamp").and_then(|v| v.as_str()).unwrap_or_default();
                                    let timestamp = chrono::DateTime::parse_from_rfc3339(timestamp_str)
                                        .map(DateTime::<Utc>::from)
                                        .unwrap_or_else(|_| Utc::now()); // Fallback timestamp
                                    let metadata_val = payload.get("metadata").cloned().unwrap_or(JsonValue::Null);

                                    results.push(QueryResultItem {
                                        id: Uuid::parse_str(&point.id.unwrap().to_string()).unwrap_or_else(|_| Uuid::new_v4()), // Handle potential parse error
                                        text,
                                        timestamp,
                                        score: point.score,
                                        metadata: metadata_val,
                                    });
                                }
                                let response = QueryMemoryResponse { results };
                                tx.send(StreamItem::ok(request.id, serde_json::to_value(response)?)).await?;
                            }
                            Err(e) => {
                                eprintln!("Error searching Qdrant: {}", e);
                                let error_item = StreamItem::error(request.id, 500, format!("Qdrant search error: {}", e));
                                tx.send(error_item).await?;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error getting query embedding from Ollama: {}", e);
                        let error_item = StreamItem::error(request.id, 500, format!("Ollama query embedding error: {}", e));
                        tx.send(error_item).await?;
                    }
                }
            }
            _ => {
                eprintln!("Unknown method: {}", request.method);
                tx.send(StreamItem::error(request.id, 404, "Method not found")).await?;
            }
        }
        Ok(())
    }
}