use std::{fmt::format, sync::{atomic::AtomicI32, Arc}, time::SystemTime};

use reagent::{init_default_tracing, Agent, AgentBuilder, McpServerType};
use rmcp::{
    handler::server::tool::{Parameters, ToolCallContext, ToolRouter}, model::{CallToolRequestParam, CallToolResult, CancelledNotification, CancelledNotificationMethod, CancelledNotificationParam, Content, Extensions, InitializeRequestParam, InitializeResult, Meta, Notification, NumberOrString, ProgressNotification, ProgressNotificationMethod, ProgressNotificationParam, ProgressToken, Request, ServerCapabilities, ServerInfo, ServerNotification}, schemars, service::{NotificationContext, RequestContext}, tool, tool_handler, tool_router, transport::{common::server_side_http::session_id, SseServer}, Peer, RoleServer, ServerHandler
};
use anyhow::Result;
use serde::{de::IntoDeserializer, Deserialize};
use tokio::sync::Mutex;
use crate::rag::Rag;

mod rag;

const BIND_ADDRESS: &str = "127.0.0.1:8005";


#[tokio::main]
async fn main() -> Result<()> {
    init_default_tracing();
    let _ = dotenv::dotenv();

    let ct = SseServer::serve(BIND_ADDRESS.parse()?)
        .await?
        .with_service(|| Service::new());

    tokio::signal::ctrl_c().await?;
    ct.cancel();

    Ok(())
}


#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StructRequest {
    pub question: String,
    pub k: u64
}

#[derive(Debug, Clone)]
struct Service {
    tool_router: ToolRouter<Service>,
}

#[tool_router]
impl Service {
    pub fn new() -> Self { 
        Self { 
            tool_router: Self::tool_router(),
        } 
    }

    #[tool(description = "Consult knowledge base. Given a question will return 'k' pessages that may contain answers. Keep questions percise with long forms of named entities.")]
    pub async fn rag_lookup(
        &self, 
        Parameters(StructRequest{question, k}): Parameters<StructRequest>,
        client: Peer<RoleServer>,
        meta: Meta
    ) -> Result<CallToolResult, rmcp::Error> {
        let start = SystemTime::now();

        let rag = Rag::default();
        let results = match rag.search_k(question, k).await {
            Ok(re) => re,
            Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        };

        let resp = results
            .chunks
            .iter()
            .map(|c| Content::text(c.content.clone()))
            .collect();

        Ok(CallToolResult::success(resp))
    }
}

#[tool_handler]
impl ServerHandler for Service {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("This is a RAG lookup service of famnit web page.".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
