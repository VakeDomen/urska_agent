use std::sync::{Arc};

use anyhow::Result;
use reagent::{init_default_tracing, Agent, Message};
use rmcp::{model::{CallToolResult, Content, ServerCapabilities, ServerInfo}, schemars, tool, transport::SseServer, ServerHandler};
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex};

use crate::{memory_store_agent::init_memory_store_agent, programme_agent::init_programme_agent, util::{get_memories, history_to_memory_prompt}};


mod programme;
mod util;
mod programme_agent;
mod memory_store_agent;

const BIND_ADDRESS: &str = "127.0.0.1:8003";
const BASE_URL: &str = "https://www.famnit.upr.si";
const MEMORY_MCP_URL: &str = "http://localhost:8002/sse";
const SCRAPER_MCP_URL: &str = "http://localhost:8000/sse";


#[tokio::main]
async fn main() -> Result<()> {
    init_default_tracing();
    // comm channel between response agent and memory agent
    let (tx, mut rx) = mpsc::channel::<Vec<Message>>(32); 
    
    
    let memory_storage_agent = Arc::new(init_memory_store_agent().await?);
    tokio::spawn(async move {
        while let Some(history) = rx.recv().await {
            let mut agent = (*memory_storage_agent).clone();
            let tools = agent.tools;
            agent.tools = None;
            agent.clear_history();

            let memory_prompt = history_to_memory_prompt(history);

            let _ = agent.invoke(&format!("{}\n\n---\n\nYour first task is to \
            identify all potential memories and nothing else. Please write a list of \
            memoris that might be usefull at some time in the future.", memory_prompt)).await;

            agent.tools = tools;

            let _ = agent.invoke("For each potential memory, check if it \
            already exists in the long term memory storage using the query_memory \
            tool. For each one determine wether it already exists and is duplicate \
            or wether it should be stored.").await;

            let _ = agent.invoke("Store the memories you determined to be \
            correct for storage. It is extremely important that the memories stored are \
            not duplicates. If the memory was seen in the query_memory tool response \
            it shoud NOT be stored again. Your main task is to not duplicate information \
            but only store new, never seen before facts.").await;

            println!("[Memory Task]: Finished processing a conversation history.");
        }
    });

    
    let agent = init_programme_agent().await?;

    let ct = SseServer::serve(BIND_ADDRESS.parse()?)
        .await?
        .with_service(move || Service::new(agent.clone(), tx.clone()));

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
    memory_queue: mpsc::Sender<Vec<Message>>,
}
#[tool(tool_box)]
impl Service {
    pub fn new(agent: Agent, memory_queue: mpsc::Sender<Vec<Message>>) -> Self {
        Self { agent: Arc::new(Mutex::new(agent)), memory_queue }
    }

    #[tool(
        description = r#"
Use this tool to ask an expert agent about the study programmes at the University of Primorska's Faculty of Mathematics, Natural Sciences and Information Technologies (UP FAMNIT).

This tool is ideal for answering specific questions about undergraduate, master's, or doctoral programmes. The agent can provide details on admission requirements, course lists, programme structure, ECTS credits, coordinators, and more.

### How to phrase your question:
- Be specific. For example, instead of asking "Tell me about computer science," ask "What are the admission requirements for the undergraduate Computer Science programme?"
- If you know the study level (e.g., "master's"), include it in the question for a more precise answer.
- Ask one clear question at a time.

### Example questions:
- "What courses are in the second year of the master's programme in Data Science?"
- "Who is the coordinator for the undergraduate Biopsychology programme?"
- "What are the main goals of the doctoral programme in Mathematical Sciences?"
"#
    )]
    pub async fn ask_programme_expert(&self, #[tool(aggr)] question: StructRequest) -> Result<CallToolResult, rmcp::Error> {
        let mut agent = self.agent.lock().await;
        agent.clear_history();

        let memory_query_args = serde_json::json!({ "query_text": question.question, "top_k": 5 });
        let initial_memory_result = match get_memories(memory_query_args).await {
            Ok(memories) => memories,
            Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())]))            
        };
        agent.history.push(Message::tool(initial_memory_result, "query_memory"));

        let resp = agent.invoke(question.question).await;
        let _ = agent.save_history("programme_conversation.json");
        let final_history = agent.history.clone();
        if let Err(e) = self.memory_queue.send(final_history).await {
            eprintln!("[ERROR] Failed to send history to memory queue: {}", e);
        }

        // let _memory_resp = agent.invoke("Is there any memory you would like to store?").await;
        Ok(CallToolResult::success(vec![Content::text(resp.unwrap().content.unwrap())]))
    }
}

#[tool(tool_box)]
impl ServerHandler for Service {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("An agent that provides information on university study programmes.".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
