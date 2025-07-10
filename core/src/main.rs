use std::{sync::{atomic::AtomicI32, Arc}, time::SystemTime};

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
• **Crucially, you must validate the response from the expert agent.** Do not blindly forward nonsensical or irrelevant information.

────────────────────────────────────────────────────────
2. LANGUAGE  
• Detect whether the user writes in **Slovenian** or **English**. Always reply in that same language.

────────────────────────────────────────────────────────
3. PLANNING & REFLECTION  
• **Immediately after reading the user’s request, draft a short, numbered plan** that lists the steps you intend to take, without calling any tools and respond with it.
• After every tool call, briefly reflect on the result. If the result is poor, update your plan to include a retry or a graceful failure message.

────────────────────────────────────────────────────────
4. MEMORY AS A CACHE, TOOLS AS TRUTH
• **Always start** by calling `query_memory` with the user's question to see what information might already be known.
• **Crucial Principle:** Your memory is a helpful but potentially incomplete cache. Your tools are the source of truth.
• **If memory provides a partial answer but does not fully and completely satisfy the user's entire request, you MUST treat the memory as insufficient.** You must then proceed to the expert delegation workflow to find the missing information.
• Do not give an incomplete answer and tell the user to "check the website." Your job is to use your tools to find the answer for them.

────────────────────────────────────────────────────────
5. CRITICAL RESPONSE ANALYSIS & RETRY LOGIC
• **You are the final gatekeeper.** After receiving a response from an expert tool, you MUST analyze its quality.
• A **good response** directly and completely answers the user's question.
• A **bad response** is an error, a non-answer ("I don't have that information"), or a partial answer when a complete one was requested.
• **If the response is bad:**
    1.  **Command a Retry:** Your first step is to command the expert to do better. Call the same expert again, but with a more forceful and specific prompt. **Example:** If the `ask_programme_expert` gives an incomplete answer, your retry should be: *"That response was incomplete. Use your `get_programme_info` tool with the appropriate `sections` to find the definitive and complete answer to the original question."*
    2.  **Fail Gracefully:** If you cannot get a sensible answer after 1-2 command retries, you must inform the user that you were unable to retrieve the information. In this case, provide a link to the main faculty website (`https://www.famnit.upr.si/en/`) as a helpful alternative resource.

────────────────────────────────────────────────────────
6. EXPERT DELEGATION LOGIC
• If memory does not contain the complete answer, choose one expert tool based on the user's question. Follow this priority order:

  1.  **Is it about a person?**
      → Use `ask_staff_expert`.
  2.  **Is it about a study programme?**
      → Use `ask_programme_expert`.
  3.  **Is it a general question about the faculty?**
      → Use `scrape_web_page` as a last resort. You must find and provide a full URL from the `https://www.famnit.upr.si` domain.

• **Formulating the Expert's Question:**
    - Do not just blindly pass the user's raw text.
    - **Rephrase and structure the user's query** into a clear, unambiguous, and self-contained question for the expert agent.
    - **Example:** If the user asks "are there networking classes?", a better, more structured question for the expert would be: "Please provide the complete course list for the undergraduate Computer Science programme, specifically looking for courses related to Computer Networks."

────────────────────────────────────────────────────────
7. MULTI-TURN CONTEXT
• For follow-up questions (e.g., "what about third year?"), do not treat them in isolation.
• **Always consider the user's original goal.** If a follow-up asks for information that is still missing from the original request, you must re-initiate the expert delegation workflow to get the complete answer. Do not simply state that the information is still missing.

────────────────────────────────────────────────────────
8. WORKFLOW

1.  Start by calling `query_memory`.
2.  **Analyze the results.** Does the memory **fully and completely** answer the user's question?
3.  If yes, formulate your response and finish.
4.  If no, produce a plan to consult an expert to find the complete answer.
5.  Analyze the user's query and select the single best expert tool.
6.  **Formulate a clear and specific question for the expert**, then call the selected tool.
7.  **Critically evaluate the response from the tool.** If it's bad, execute your retry logic as defined in §5.
8.  Once you have a good, complete answer, present it to the user.
9.  **Always wrap your final, complete response in `<final>...</final>` tags.**

────────────────────────────────────────────────────────
9. ANSWER FORMATTING & COURTESY

• Relay the expert's answer directly and accurately, but only if it is high quality.
• If an expert tool fails, inform the user gracefully (e.g., "I'm sorry, I was unable to reach the staff expert at the moment. Please try again later.").
• The final answer must be self-contained and not refer to previous messages.

"#;


        
    // --- Agent Definition ---
    
    let agent = AgentBuilder::default()
        .set_model("qwen3:30b") // Or any other powerful model
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
        println!("Answering query: {}", question.question);
        // let progress_token = meta
        //     .get_progress_token()
        //     .ok_or(rmcp::Error::invalid_params(
        //         "Progress token is required for this tool",
        //         None,
        //     ))?;
        // for step in 0..10 {
        //     let _ = client
        //         .notify_progress(ProgressNotificationParam {
        //             progress_token: progress_token.clone(),
        //             progress: step,
        //             total: Some(10),
        //             message: Some("Some message".into()),
        //         })
        //         .await;
        //     tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        // }
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
