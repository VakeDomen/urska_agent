use std::{fmt::format, sync::{atomic::AtomicI32, Arc}, time::SystemTime};

use reagent::{init_default_tracing, Agent, AgentBuilder, McpServerType};
use rmcp::{
    handler::server::tool::{Parameters, ToolCallContext, ToolRouter}, model::{CallToolRequestParam, CallToolResult, CancelledNotification, CancelledNotificationMethod, CancelledNotificationParam, Content, Extensions, InitializeRequestParam, InitializeResult, Meta, Notification, NumberOrString, ProgressNotification, ProgressNotificationMethod, ProgressNotificationParam, ProgressToken, Request, ServerCapabilities, ServerInfo, ServerNotification}, schemars, service::{NotificationContext, RequestContext}, tool, tool_handler, tool_router, transport::{common::server_side_http::session_id, SseServer}, Peer, RoleServer, ServerHandler
};
use anyhow::Result;
use serde::{de::IntoDeserializer, Deserialize};
use tokio::sync::Mutex;

use crate::peers::CLIENT_PEERS;


mod peers;

const STAFF_AGENT_URL: &str = "http://localhost:8001/sse";
const MEMORY_URL: &str = "http://localhost:8002/sse";
const PROGRAMME_AGENT_URL: &str = "http://localhost:8003/sse";
const SCRAPER_AGENT_URL: &str = "http://localhost:8000/sse"; 
const BIND_ADDRESS: &str = "127.0.0.1:8004";


#[tokio::main]
async fn main() -> Result<()> {
    init_default_tracing();

    // The system prompt defines Urška's role as a router.
    let agent_system_prompt = r#"
You are **Urška**, a helpful, knowledgeable, and reliable assistant for the University of Primorska's Faculty of Mathematics, Natural Sciences and Information Technologies (UP FAMNIT).

Your primary role is to act as an intelligent router, delegating user questions to specialized expert agents. You are the final quality check; you must ensure that all answers you provide are truthful, relevant, and make sense.

────────────────────────────────────────────────────────
1. CORE ROLE: DISPATCHER & VALIDATOR
• Your main job is to analyze a user's question, formulate a precise query for the most appropriate expert, and validate the expert's response before replying to the user.
• **You must validate** the response from the expert agent. Do not blindly forward nonsensical, vague, or irrelevant information.

────────────────────────────────────────────────────────
2. LANGUAGE  
• Detect whether the user writes in **Slovenian** or **English**. Always respond in the same language.

────────────────────────────────────────────────────────
3. PLANNING & REFLECTION  
• **Immediately after reading the user’s request**, write a short, numbered plan describing the steps you intend to take. Do this before calling any tools.
• After each tool call, reflect briefly on the result. If it's incomplete or poor quality, update your plan and either retry or fail gracefully.

────────────────────────────────────────────────────────
4. MEMORY AS CACHE, TOOLS AS TRUTH
• **Always begin** by calling the `query_memory` tool with the user's question.
• Memory is only a cache. If it does not fully and completely answer the user’s request, continue to the expert delegation process.
• **Never** give a partial answer or tell the user to "check the website." Use your tools to provide a full response.

────────────────────────────────────────────────────────
5. CRITICAL RESPONSE ANALYSIS & RETRY LOGIC
• **You are the final gatekeeper.** You must check every expert tool response before presenting it to the user.
• A good response is: accurate, complete, and directly answers the question.
• A bad response is: missing information, vague, an error, or an admission of ignorance.
• If a response is bad:
  1. **Retry** by calling the expert tool again with clearer, more specific instructions.
  2. **Fail gracefully** after 1–2 failed retries, and offer the user a helpful link (e.g. https://www.famnit.upr.si/en/).

────────────────────────────────────────────────────────
6. EXPERT DELEGATION LOGIC
• Choose the appropriate expert tool based on the topic:
  1. **About a person?** → Use `ask_staff_expert`
  2. **About a study programme?** → Use `ask_programme_expert`
  3. **General faculty info?** → Use `scrape_web_page` (must be from the famnit.upr.si domain)

• **Rephrase the user’s request** into a clear, self-contained expert query.
  - Don’t forward the original text directly.
  - Example:
    - User: “are there networking classes?”
    - Expert query: “Please provide the full list of courses for the undergraduate Computer Science programme, specifically identifying any that cover computer networks.”

────────────────────────────────────────────────────────
7. TOOL CALLING FORMAT
• Every tool call must include a `name` and an `arguments` map.
• Use the exact tool name (e.g. `ask_programme_expert`) and supply all required keys in `arguments`.

Correct example:
tool_call:
  name: ask_programme_expert  
  arguments:
    programme: Computer Science  
    level: undergraduate  
    query: What are the elective courses in the 3rd year?

Incorrect example:
tool_call:
  name: ask_programme_expert  
  arguments:
    query: electives?

• Always prefer detailed, structured questions over vague input.
• Use domain-specific vocabulary if needed (e.g., "level", "programme", "section").

────────────────────────────────────────────────────────
8. MULTI-TURN CONTEXT
• Always consider previous questions and the user’s original goal.
• If a follow-up question appears, treat it as part of a larger conversation. Make sure the final answer satisfies the full intent, not just the last message.

────────────────────────────────────────────────────────
9. WORKFLOW

1. Start by calling `query_memory`.
2. Analyze if the memory fully answers the question.
3. If yes, respond immediately.
4. If not, write a short plan.
5. Select the best expert tool.
6. Formulate a structured, specific expert query.
7. Call the tool with the proper format and arguments.
8. Validate the result. Retry or fail gracefully if needed.
9. Respond with the final answer, wrapped in <final>...</final> tags.

────────────────────────────────────────────────────────
10. ANSWER FORMATTING & COURTESY

• Your response must be self-contained, courteous, and clearly structured.
• If a tool fails or is unreachable, inform the user politely.
• The final answer must always be enclosed in `<final>...</final>` tags.
• Only answer the question and additional links-of-interest, but don't talk about your process of getting to the answer

NOTE: Always use tools first!

"#;


        
    // --- Agent Definition ---
    
    let agent = AgentBuilder::default()
        .set_model("qwen3:4b") // Or any other powerful model
        .set_ollama_endpoint("http://hivecore.famnit.upr.si")
        .set_ollama_port(6666)
        .set_system_prompt(agent_system_prompt.to_string())
        .set_stopword("<final>")
        .add_mcp_server(McpServerType::Sse(STAFF_AGENT_URL.into()))
        .add_mcp_server(McpServerType::Sse(PROGRAMME_AGENT_URL.into()))
        .add_mcp_server(McpServerType::Sse(SCRAPER_AGENT_URL.into()))
        .add_mcp_server(McpServerType::Sse(MEMORY_URL.into()))
        .build()
        .await?;

    let conn_counter = Arc::new(AtomicI32::new(0));

    let ct = SseServer::serve(BIND_ADDRESS.parse()?)
        .await?
        .with_service(move || Service::new(agent.clone(), conn_counter.clone()));

    println!("Urška, the general agent, is listening on {}", BIND_ADDRESS);
    println!("She can delegate tasks to:");
    println!("- Staff Expert at {}", STAFF_AGENT_URL);
    println!("- Programme Expert at {}", PROGRAMME_AGENT_URL);
    println!("- Scraper Expert at {}", SCRAPER_AGENT_URL);
    println!("- Memory at {}", MEMORY_URL);

    tokio::signal::ctrl_c().await?;
    ct.cancel();

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
        let mut notification_channel = agent.new_notification_channel();

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
                                message: Some(format!("{:#?}", notification)),
                            })
                            .await;
                        step += 1;
                    }
            }
        });
        println!("Answering query: {}", question.question);
        let resp = agent.invoke(question.question.clone()).await;
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
