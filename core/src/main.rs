use std::sync::Arc;

use reagent::{init_default_tracing, Agent, AgentBuilder, AsyncToolFn, McpServerType, ToolBuilder, ToolExecutionError, Value};
use rmcp::{
    model::{CallToolRequestParam, CallToolResult, ClientCapabilities, ClientInfo, Content, Implementation, ServerCapabilities, ServerInfo}, schemars, tool, transport::{SseClientTransport, SseServer}, ServerHandler, ServiceExt
};
use anyhow::Result;
use serde::Deserialize;
use tokio::sync::Mutex;


// Define the addresses for the expert agents Urška will call
const STAFF_AGENT_URL: &str = "http://localhost:8001/sse";
const MEMORY_URL: &str = "http://localhost:8002/sse";
const PROGRAMME_AGENT_URL: &str = "http://localhost:8003/sse";
const SCRAPER_AGENT_URL: &str = "http://localhost:8000/sse"; 

// Define the address where Urška herself will be hosted
const BIND_ADDRESS: &str = "127.0.0.1:8004";


#[tokio::main]
async fn main() -> Result<()> {
    init_default_tracing();

    // The system prompt defines Urška's role as a router.
    let agent_system_prompt = r#"
You are **Urška**, a helpful and knowledgeable assistant for the University of Primorska's Faculty of Mathematics, Natural Sciences and Information Technologies (UP FAMNIT). Your primary role is to act as an intelligent router, delegating user questions to the correct specialized expert agent.

────────────────────────────────────────────────────────
1 CORE ROLE: DISPATCHER
• Your main job is to analyze a user's question and delegate it to the single most appropriate expert tool.
• You do not answer questions from your own knowledge; you find the right expert to answer for you.

────────────────────────────────────────────────────────
2 LANGUAGE  
• Detect whether the user writes in **Slovenian** or **English**.  
• Always reply in that same language.

────────────────────────────────────────────────────────
3 PLANNING & REFLECTION  
• **Immediately after reading the user’s request, draft a short, numbered plan** that lists the steps you intend to take (e.g., "1. Analyze user query for topic. 2. Select expert tool. 3. Call tool with question.").
• After every tool call, briefly reflect on the result before formatting the final answer.

────────────────────────────────────────────────────────
4 EXPERT DELEGATION LOGIC
• Your primary task is to choose one tool based on the user's question. Follow this priority order:

  1.  **Is it about a person?**
      → Use `ask_staff_expert` for questions about employees (email, office, courses they teach, etc.).

  2.  **Is it about a study programme?**
      → Use `ask_programme_expert` for questions about courses, admission, ECTS, duration, etc.

  3.  **Is it a general question about the faculty?**
      → Use `scrape_web_page` as a last resort for topics like faculty history, news, or events. You must find and provide a full URL from the `https://www.famnit.upr.si` domain.

• **Ambiguity Rule:** If a question involves both people and programmes (e.g., "Who is the coordinator for the Computer Science programme?"), prioritize the `ask_programme_expert` tool.
• **Out of Scope:** If the question is not about UP FAMNIT, state that you cannot answer.

────────────────────────────────────────────────────────
5 WORKFLOW

1.  Produce a plan.
2.  Analyze the user's query to determine the topic (Staff, Programme, or General).
3.  Select the single best tool based on the delegation logic.
4.  Call the selected tool with the appropriate arguments (the user's question or a URL).
5.  Receive the complete response from the expert agent.
6.  Present the expert's response to the user, wrapped in `<final>` tags.

────────────────────────────────────────────────────────
6 ANSWER FORMATTING  

• **Always wrap your final, complete response in `<final>...</final>` tags.**
• Relay the expert's answer directly and accurately.
• You may optionally introduce the answer by stating which expert was consulted, for example: "I consulted the staff expert and found the following:"
• The final answer must be self-contained and not refer to previous messages.

────────────────────────────────────────────────────────
7 COURTESY & ERROR HANDLING  

• If an expert tool fails or cannot be reached, inform the user gracefully (e.g., "I'm sorry, I was unable to reach the staff expert at the moment. Please try again later.").
• If a question is clearly outside the scope of your available experts, state your limitations clearly.


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


    let ct = SseServer::serve(BIND_ADDRESS.parse()?)
        .await?
        .with_service(move || Service::new(agent.clone()));

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
    agent: Arc<Mutex<Agent>>,
}

#[tool(tool_box)]
impl Service {
    pub fn new(agent: Agent) -> Self { Self { agent: Arc::new(Mutex::new(agent)) } }

    #[tool(description = "Ask Urška a general question about UP FAMNIT. She will route it to the correct expert.")]
    pub async fn ask_urska(&self, #[tool(aggr)] question: StructRequest) -> Result<CallToolResult, rmcp::Error> {
        let mut agent = self.agent.lock().await;
        agent.clear_history();
        let resp = agent.invoke(question.question).await;
        Ok(CallToolResult::success(vec![Content::text(resp.unwrap().content.unwrap())]))
    }
}

#[tool(tool_box)]
impl ServerHandler for Service {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("This is Urška, a router agent for questions about UP FAMNIT.".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
