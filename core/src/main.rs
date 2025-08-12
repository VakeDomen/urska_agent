use std::{fmt::format, sync::{atomic::AtomicI32, Arc}, time::SystemTime};

use reagent::{init_default_tracing, Agent, AgentBuilder, McpServerType};
use rmcp::{
    handler::server::tool::{Parameters, ToolCallContext, ToolRouter}, model::{CallToolRequestParam, CallToolResult, CancelledNotification, CancelledNotificationMethod, CancelledNotificationParam, Content, Extensions, InitializeRequestParam, InitializeResult, Meta, Notification, NumberOrString, ProgressNotification, ProgressNotificationMethod, ProgressNotificationParam, ProgressToken, Request, ServerCapabilities, ServerInfo, ServerNotification}, schemars, service::{NotificationContext, RequestContext}, tool, tool_handler, tool_router, transport::{common::server_side_http::session_id}, Peer, RoleServer, ServerHandler
};
use anyhow::Result;
use serde::{de::IntoDeserializer, Deserialize};
use tokio::sync::Mutex;

use rmcp::transport::streamable_http_server::{
    StreamableHttpService, session::local::LocalSessionManager,
};

use crate::usrka::build_urska;

mod usrka;
mod executor;
mod replanner;
mod blueprint;
mod planner;
mod prompt_reconstuct;
mod quick_responder;

const STAFF_AGENT_URL: &str = "http://localhost:8001/mcp";
const MEMORY_URL: &str = "http://localhost:8002/mcp";
const PROGRAMME_AGENT_URL: &str = "http://localhost:8003/mcp";
const SCRAPER_AGENT_URL: &str = "http://localhost:8000/sse"; 
const RAG_PAGE_SERVICE: &str = "http://localhost:8005/mcp"; 
const RAG_RULES_SERVICE: &str = "http://localhost:8006/mcp"; 
const RAG_FAQ_SERVICE: &str = "http://localhost:8007/mcp"; 
const BIND_ADDRESS: &str = "127.0.0.1:8004";


#[tokio::main]
async fn main() -> Result<()> {
    init_default_tracing();

    // The system prompt defines Urška's role as a router.
    let agent_system_prompt = r#"
You are **Urška**, a helpful, knowledgeable, and reliable assistant for the University of Primorska's Faculty of Mathematics, Natural Sciences and Information Technologies (UP FAMNIT).

1. LANGUAGE  
• Detect whether the user writes in **Slovenian** or **English**. Always respond in the same language.

2. ANSWER FORMATTING
• Use Markdown for clear presentation (lists, tables).
• Always specify the programme level in your answer (e.g., "The undergraduate programme in Mathematics...").
• Do not use 'etc.'; provide the full answer.
• If the tool provides a source URL, always include it in your response.


"#;


        
    // --- Agent Definition ---
    
    let agent = build_urska().await?;

    let conn_counter = Arc::new(AtomicI32::new(0));

    let service = StreamableHttpService::new(
        move || Ok(Service::new(agent.clone(), conn_counter.clone())),
        LocalSessionManager::default().into(),
        Default::default(),
    );

    let router = axum::Router::new().nest_service("/mcp", service);
    let tcp_listener = tokio::net::TcpListener::bind(BIND_ADDRESS).await?;
    let _ = axum::serve(tcp_listener, router)
        .with_graceful_shutdown(async { tokio::signal::ctrl_c().await.unwrap() })
        .await;

    println!("Urška, the general agent, is listening on {}", BIND_ADDRESS);
    println!("She can delegate tasks to:");
    println!("- Staff Expert at {}", STAFF_AGENT_URL);
    println!("- Programme Expert at {}", PROGRAMME_AGENT_URL);
    println!("- Scraper Expert at {}", SCRAPER_AGENT_URL);
    println!("- Memory at {}", MEMORY_URL);

    tokio::signal::ctrl_c().await?;

    Ok(())
}


#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StructRequest {
    pub question: String,
}

#[derive(Debug, Clone)]
struct Service {
    id: String,
    agent: Arc<Mutex<Agent>>,
    tool_router: ToolRouter<Service>,
}

#[tool_router]
impl Service {
    pub fn new(agent: Agent, conn_counter: Arc<AtomicI32>) -> Self { 
        let num = conn_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self { 
            agent: Arc::new(Mutex::new(agent)), 
            id: format!("{num}") ,
            tool_router: Self::tool_router(),
        } 
    }

    #[tool(description = "Ask Urška a general question about UP FAMNIT. She will route it to the correct expert.")]
    pub async fn ask_urska(
        &self, 
        Parameters(question): Parameters<StructRequest>,
        client: Peer<RoleServer>,
        meta: Meta
    ) -> Result<CallToolResult, rmcp::Error> {
        let start = SystemTime::now();
        let mut agent = self.agent.lock().await;
        let mut notification_channel = match agent.new_notification_channel().await {
            Ok(ch) => ch,
            Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())]))
        };

        tokio::spawn(async move {
            if let Ok(progress_token) =  meta
                .get_progress_token()
                .ok_or(rmcp::Error::invalid_params(
                    "Progress token is required for this tool",
                    None,
                )) {
                    let mut step = 1;
                    while let Some(notification) = notification_channel.recv().await {
                        let _ = client
                            .notify_progress(ProgressNotificationParam {
                                progress_token: progress_token.clone(),
                                progress: step,
                                total: None,
                                message: serde_json::to_string(&notification).ok(),
                            })
                            .await;
                        step += 1;
                    }
            }
        });
        println!("Answering query: {}", question.question);
        let resp = agent.invoke_flow(question.question.clone()).await;
        let file_name = format!("{}_conversation.json", self.id);
        let _ = agent.save_history(file_name);
        println!("Time to answe query: {:?} | {}", start.elapsed(), question.question);
        Ok(CallToolResult::success(vec![Content::text(resp.unwrap().content.unwrap())]))
    }
}

#[tool_handler]
impl ServerHandler for Service {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("This is Urška, a router agent for questions about UP FAMNIT.".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
