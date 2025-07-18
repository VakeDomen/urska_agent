use std::sync::Arc;

use reagent::{init_default_tracing, Agent, Message};
use rmcp::{
    handler::server::tool::{Parameters, ToolRouter}, model::{CallToolResult, Content, Meta, ProgressNotificationParam, ServerCapabilities, ServerInfo}, schemars, tool, tool_handler, tool_router, transport::SseServer, Peer, RoleServer, ServerHandler
};
use anyhow::Result;
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex};

use crate::{
    memory_store_agent::init_memory_store_agent, profile::StaffProfile, staff_agent::init_staff_agent, util::{get_memories, get_page, history_to_memory_prompt, staff_html_to_markdown}
};

mod profile;
mod util;
mod memory_store_agent;
mod staff_agent;

const BIND_ADDRESS: &str = "127.0.0.1:8001";
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

            let _ = agent.invoke_flow(&format!("{}\n\n---\n\nYour first task is to \
            identify all potential memories and nothing else. Please write a list of \
            memoris that might be usefull at some time in the future.", memory_prompt)).await;

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

    let agent = init_staff_agent().await?;
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



#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ProfileRequest {
    pub name: String,
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

    // #[tool(
    //     description = r#"
    //     Given a name, provides a profile of employees/staff
    //     "#
    // )]
    // pub async fn get_staff_profile(
    //     &self, 
    //     Parameters(prof): Parameters<ProfileRequest>,
    // ) -> Result<CallToolResult, rmcp::Error> {
    //     let staff_list_result = get_page("https://www.famnit.upr.si/en/about-faculty/staff/").await;
    //     let all_staff = match staff_list_result {
    //         Ok(staff_list) => staff_html_to_markdown(&staff_list),
    //         Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())])) 
    //     };
    //     let names = all_staff
    //         .clone()
    //         .keys()
    //         .map(|k| k.to_string())
    //         .collect::<Vec<String>>();

    //     let top_names = crate::util::rank_names(names, &prof.name)[0..1 as usize].to_vec();
    //     println!("Top names: {:#?}", top_names);
    //     let mut result = "# Profiles \n\n ---\n\n".to_string();

    //     for name in top_names {
    //         let profile_page_link = all_staff.get(&name);
    //         if profile_page_link.is_none() {
    //             continue;
    //         }
    //         let profile_page_link = profile_page_link.unwrap();
    //         let profile_page = get_page(profile_page_link).await;

    //         if profile_page.is_err() {
    //             continue;
    //         }
    //         let profile_page = profile_page.unwrap();
    //         let profile = StaffProfile::from(profile_page);

    //         result = format!("{} \n\n --- \n\n {}", result, profile.to_string());

    //     }
    //     // let _memory_resp = agent.invoke("Is there any memory you would like to store?").await;
    //     Ok(CallToolResult::success(vec![Content::text(result)]))
    // }

    #[tool(
        description = r#"
        Use this tool to ask an expert agent about employees at the University of Primorska's Faculty of Mathematics, Natural Sciences and Information Technologies (UP FAMNIT).

        This tool is ideal for finding specific information about staff members, including their office location, phone number, email address, department, research fields, and the courses they teach.

        ### How to phrase your question:
        - Use the full name of the employee if you know it for the most accurate results.
        - Be specific about the information you need. For example, ask "What is their office number?"
        - Ask one clear question at a time.

        ### Example questions:
        - "What is the email address for Domen Vake?"
        - "Which courses does Janez Novak teach?"
        - "What is the office location and phone number for dr. Branko Kavšek?"
        "#
    )]
    pub async fn ask_staff_expert(
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
            instructions: Some("An agent about employees at the University of Primorska's Faculty of Mathematics, Natural Sciences and Information Technologies (UP FAMNIT)".into()),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            ..Default::default()
        }
    }
}

