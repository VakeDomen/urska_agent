use std::fmt;
// src/mcp_service.rs
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use rmcp::model::{Implementation, ProtocolVersion};
use rmcp::{
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, ServerHandler,
};
use crate::models::{
    MemoryItem, AddMemoryParams, AddMemoryResponse,
    QueryMemoryParams, QueryMemoryResponse,
};
use crate::ollama_service::OllamaService;
use crate::qdrant_service::QdrantService;

const STM_CAPACITY: usize = 20;

// AppState to hold shared resources
#[derive(Debug, Clone)]
pub struct AppState {
    stm: Arc<Mutex<VecDeque<MemoryItem>>>, // Short-Term Memory (text-based for now)
    ollama: OllamaService,
    qdrant: QdrantService,
}

impl AppState {
    pub fn new(ollama: OllamaService, qdrant: QdrantService) -> Self {
        Self {
            stm: Arc::new(Mutex::new(VecDeque::with_capacity(STM_CAPACITY))),
            ollama,
            qdrant,
        }
    }
}

impl fmt::Display for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stm_lock = self.stm.lock();
        let stm_count = match stm_lock {
            Ok(guard) => guard.len(),
            Err(_) => {
                // Handle the case where the lock is poisoned.
                // You might log this or return an error,
                // but for Display, often a placeholder is used.
                return write!(f, "AppState (STM: <Error: Lock Poisoned> | Ollama: ... | Qdrant: ...)");
            }
        };

        writeln!(f, "AppState Overview:")?;
        writeln!(f, "  Short-Term Memory (STM): {} items", stm_count)?;
        Ok(())
    }
}


#[derive(Debug, Clone)]
pub struct MemoryMcpService {
    state: Arc<AppState>,
}

// Implement the tools
#[tool(tool_box)]
impl MemoryMcpService {
    pub fn new(state: Arc<AppState>) -> Self {
        println!("MemoryMcpService instance created.");
        Self { state }
    }


    #[tool(description = "Stores a memory into the agent's long-term memory. Use this to remember facts, details, or context for future recall. Provide the 'memory' to remember.")]
    pub async fn store_memory(
        &self,
        #[tool(aggr)] params: AddMemoryParams,
    ) -> Result<CallToolResult, rmcp::Error> {
        println!("Tool 'store_memory' called with text: {:.20}...", params.memory);
        let mut memory_item = MemoryItem::new(params.memory.clone());

        // 1. (Optional) Add to STM (Short-Term Memory) - simple text store for now
        {
            let mut stm_guard = self.state.stm.lock().expect("STM lock poisoned");
            if stm_guard.len() >= STM_CAPACITY {
                stm_guard.pop_front();
            }
            stm_guard.push_back(memory_item.clone()); // Store a clone
            println!("Added to STM: {}", memory_item.id);
        }

        // 2. Get embedding from Ollama
        let embedding = self.state.ollama.get_embedding(&params.memory).await
            .map_err(|e| rmcp::Error::internal_error(format!("Ollama error: {}", e.to_string()), None))?;
        memory_item.embedding = Some(embedding);

        // 3. Store in Qdrant (LTM)
        self.state.qdrant.add_memory(&memory_item).await
            .map_err(|e| rmcp::Error::internal_error(format!("Qdrant error: {}", e.to_string()), None))?;  
        println!("Added to LTM (Qdrant): {}", memory_item.id);

        let response = AddMemoryResponse {
            id: memory_item.id.to_string(),
            status: "Memory added successfully".to_string(),
            timestamp: memory_item.timestamp,
        };

        let response_json = serde_json::to_value(response)
            .map_err(|e| rmcp::Error::internal_error(format!("Serealization error: {}", e.to_string()), None))?;  

        Ok(CallToolResult::success(vec![Content::json(response_json)?]))
    }

    #[tool(description = "Retrieves relevant memories from the agent's long-term memory based on a natural language query. It uses semantic search to find information that is contextually similar to your query. Provide 'query_text' for your search. Optionally, specify 'top_k' for the number of results (default is 5).")]
    pub async fn query_memory(
        &self,
        #[tool(aggr)] params: QueryMemoryParams,
    ) -> Result<CallToolResult, rmcp::Error> {
        println!("Tool 'query_memory' called with query: {:.20}...", params.query_text);
        // 1. Get embedding for the query text
        let query_embedding = self.state.ollama.get_embedding(&params.query_text).await
            .map_err(|e| rmcp::Error::internal_error(format!("Ollama embedding error: {}", e.to_string()), None))?;  

        // 2. Search Qdrant
        let search_results = self.state.qdrant.search_memories(query_embedding, params.top_k).await
            .map_err(|e| rmcp::Error::internal_error(format!("Qdrant search error: {}", e.to_string()), None))?;  
        let search_results = search_results.iter()
            .map(|r| r.text.clone())
            .collect::<Vec<String>>();
        let response = QueryMemoryResponse { results: search_results };
        let response_json = serde_json::to_value(response)
            .map_err(|e| rmcp::Error::internal_error(format!("Serealization error: {}", e.to_string()), None))?;  

        Ok(CallToolResult::success(vec![Content::json(response_json)?]))
    }
}

// Implement ServerHandler to provide server information
#[tool(tool_box)]
impl ServerHandler for MemoryMcpService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "This service manages the agent's memory. \
                Stores a memory into the agent's long-term memory. Use this to remember facts, details, or context for future recall. Provide the 'memory' to remember. \
                Use the 'query_memory' tool to retrieve information from memory using a natural language query; it performs a semantic search to find the most relevant stored memories.".to_string()
            ),
        }
    }

}