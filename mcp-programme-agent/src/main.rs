use std::sync::{Arc};

use anyhow::Result;
use reagent::{init_default_tracing, util::invocations::invoke_without_tools, Agent, Message};
use rmcp::{handler::server::tool::{Parameters, ToolRouter}, model::{CallToolResult, Content, Meta, ProgressNotificationParam, ServerCapabilities, ServerInfo}, schemars, tool, tool_handler, tool_router, transport::SseServer, Peer, RoleServer, ServerHandler};
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex};

use crate::{memory_store_agent::init_memory_store_agent, programme_agent::init_programme_agent, util::{get_memories, history_to_memory_prompt}};


mod programme;
mod util;
mod programme_agent;
mod memory_store_agent;

const BIND_ADDRESS: &str = "127.0.0.1:8003";
const BASE_URL: &str = "https://www.famnit.upr.si";
const MEMORY_MCP_URL: &str = "http://localhost:8002/mcp";
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

            // let _ = agent.invoke_flow(&format!("{}\n\n---\n\nYour first task is to \
            // identify all potential memories and nothing else. Please write a list of \
            // memoris that might be usefull at some time in the future.", memory_prompt)).await;

            agent.history.push(Message::user(format!("{}\n\n---\n\nYour first task is to \
            identify all potential memories and nothing else. Please write a list of \
            memoris that might be usefull at some time in the future.", memory_prompt)));
            let resp = match invoke_without_tools(&mut agent).await {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[ERROR] Failed to invoke memory agent: {}", e);
                    return
                }
            };
            agent.history.push(Message::assistant(resp.message.content.unwrap_or("".into())));


            agent.tools = tools;

            let _ = agent.invoke_flow("For each potential memory, check if it \
            already exists in the long term memory storage using the query_memory \
            tool. For each one determine wether it already exists and is duplicate \
            or wether it should be stored.").await;

            let _ = agent.invoke_flow("Store the memories you determined to be \
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
    tool_router: ToolRouter<Service>
}

#[tool_router]
impl Service {
    pub fn new(agent: Agent, memory_queue: mpsc::Sender<Vec<Message>>) -> Self {
        Self { 
            agent: Arc::new(Mutex::new(agent)), 
            memory_queue,
            tool_router: Self::tool_router()
        }
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
    pub async fn ask_programme_expert(
        &self, 
        Parameters(question): Parameters<StructRequest>,
        client: Peer<RoleServer>,
        meta: Meta
    ) -> Result<CallToolResult, rmcp::Error> {
        let mut agent = self.agent.lock().await;
        agent.clear_history();
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

        let memory_query_args = serde_json::json!({ "query_text": question.question, "top_k": 5 });
        let initial_memory_result = match get_memories(memory_query_args).await {
            Ok(memories) => memories,
            Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())]))            
        };
        agent.history.push(Message::tool(initial_memory_result, "query_memory"));

        let resp = agent.invoke_flow(question.question).await;
        let _ = agent.save_history("programme_conversation.json");
        let final_history = agent.history.clone();
        if let Err(e) = self.memory_queue.send(final_history).await {
            eprintln!("[ERROR] Failed to send history to memory queue: {}", e);
        }

        // let _memory_resp = agent.invoke("Is there any memory you would like to store?").await;
        Ok(CallToolResult::success(vec![Content::text(resp.unwrap().content.unwrap())]))
    }
}

#[tool_handler]
impl ServerHandler for Service {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("An agent that provides information on university study programmes.".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
