use std::{collections::HashMap, fmt::format, sync::Arc};

use reagent::{init_default_tracing, Agent, AgentBuilder, AsyncToolFn, McpServerType, Message, ToolBuilder, ToolExecutionError, Value};
use rmcp::{
    model::{CallToolRequestParam, CallToolResult, ClientCapabilities, ClientInfo, Content, Implementation, ServerCapabilities, ServerInfo}, schemars, tool, transport::{SseClientTransport, SseServer}, ServerHandler, ServiceExt
};
use anyhow::Result;
use scraper::{Html, Selector};
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex};

use crate::{profile::StaffProfile, util::{history_to_memory_prompt, rank_names}};

mod profile;
mod util;


const BIND_ADDRESS: &str = "127.0.0.1:8001";
const MEMORY_MCP_URL: &str = "http://localhost:8002/sse";
const SCRAPER_MCP_URL: &str = "http://localhost:8000/sse";


#[tokio::main]
async fn main() -> Result<()> {
    init_default_tracing();

    // comm channel between response agent and memory agent
    let (tx, mut rx) = mpsc::channel::<Vec<Message>>(32); 

    let memory_storage_agent_prompt = r#"
    You are a meticulous librarian agent. Your sole purpose is to analyze conversation summaries and store new, important facts without creating duplicates.

    You will be given tasks sequentially. Follow each instruction precisely using the principles below.

    ### Core Principles:

    * **Be Selective:** Only identify or store facts that are specific, objective, and lasting (e.g., "Domen Vake teaches Programming III."). Do not process conversational filler, questions, or temporary information.
    * **Prevent Duplicates:** This is your most important rule. You must **never** store a fact if it is already in the long-term memory.

    ### Your Task:

    Your current task is described in the user's prompt. Execute it according to the principles and tool protocols defined above.

    "#;

    let agent_system_prompt = r#"
    You are **UniStaff-Agent**, a focused assistant that answers questions about university employees on *famnit.upr.si*.

    ────────────────────────────────────────────────────────
    1 LANGUAGE  
    • Detect whether the user writes in **Slovenian** or **English** and reply in that language.

    ────────────────────────────────────────────────────────
    2 PLANNING & REFLECTION  
    • **Immediately after reading the user’s request, draft a short, numbered plan.**
    • After every tool call, **reflect** on your progress and update the plan.

    ────────────────────────────────────────────────────────
    3 MEMORY-FIRST, BUT VERIFY
    • At the start of the conversation, you will find the results of a `query_memory` call already in your history. **Review these results first** to inform your plan.
    • **Crucial Principle:** Your memory is a helpful starting point, but it can be incomplete. The tools are the source of truth.
    • If the user asks for a list or a count of items (e.g., "list all courses they teach"), you **must still use `get_staff_profiles` to fetch the complete, definitive list** before answering.

    ────────────────────────────────────────────────────────
    4 TOOLS – OVERVIEW
    • `get_staff_profiles`: Your primary tool for getting all details about a person.
    • `get_web_page_content`: For fetching non-profile URLs.

    ────────────────────────────────────────────────────────
    5 WORKFLOW
    1.  Review the initial memory results in your history.
    2.  Produce a plan.
    3.  Handle any name ambiguity by asking for clarification if necessary.
    4.  **Call `get_staff_profiles` to get the complete and authoritative information.** Do this even if your memory has a partial answer.
    5.  Formulate your final answer using the **complete information from the tool output.**
    6.  Wrap your final, self-contained answer in `<final> … </final>`.

    "#;
    

    let memory_storage_agent = Arc::new(AgentBuilder::default()
        .set_model("qwen3:30b")
        .set_ollama_endpoint("http://hivecore.famnit.upr.si")
        .set_ollama_port(6666)
        .set_system_prompt(memory_storage_agent_prompt)
        .add_mcp_server(McpServerType::sse(MEMORY_MCP_URL)) // Connect to the memory server
        .build()
        .await?);

    // Spawn the background task that listens on the queue
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


    let staff_list_result = get_page("https://www.famnit.upr.si/en/about-faculty/staff/").await;
    let all_staff = match staff_list_result {
        Ok(staff_list) => staff_html_to_markdown(&staff_list),
        Err(e) => return Err(anyhow::anyhow!("Fetching employee list error: {:#?}", e.to_string()))
    };

    let all_names_clone = all_staff.clone();
    let similar_names_executor: AsyncToolFn = {
        Arc::new(move |args: Value| {
            let names = all_names_clone
                .clone()
                .keys()
                .map(|k| k.to_string())
                .collect::<Vec<String>>();
            Box::pin(async move {
                let names = names.clone();
                
                let query_name = args.get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolExecutionError::ArgumentParsingError("Missing 'name' argument".into()))?;

                let k = args.get("k")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| 5);



                let names = rank_names(names, query_name)[0..k as usize].to_vec();
                Ok(names.join(" \n - "))
            })
        })
    };


    let profile_executor: AsyncToolFn = {
        Arc::new(move |args: Value| {
            let names = all_staff.clone();
            Box::pin(async move {
                let profiles = names.clone();
                let names = names
                    .clone()
                    .keys()
                    .map(|k| k.to_string())
                    .collect::<Vec<String>>();
                let query_name = args.get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolExecutionError::ArgumentParsingError("Missing 'name' argument".into()))?;

                let k = args.get("k")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| 1);

                let top_names = rank_names(names, query_name)[0..k as usize].to_vec();

                let mut result = "# Profiles \n\n ---\n\n".to_string();

                for name in top_names {
                    let profile_page_link = profiles.get(&name);
                    if profile_page_link.is_none() {
                        continue;
                    }
                    let profile_page_link = profile_page_link.unwrap();
                    let profile_page = get_page(profile_page_link).await;

                    if profile_page.is_err() {
                        continue;
                    }
                    let profile_page = profile_page.unwrap();
                    let profile = StaffProfile::from(profile_page);

                    result = format!("{} \n\n --- \n\n {}", result, profile.to_string());
   
                }
                Ok(result)
            })
        })
    };


    let staff_profiles_tool = ToolBuilder::new()
        .function_name("get_staff_profiles")
        .function_description(
            "Return detailed staff-profile(s) in Markdown.\n\
             • Use when the user asks for full information (office, phone, courses…)\n\
             • Pass the query string as **name**; fuzzy match picks the best entries.\n\
             • Optional **k** (default 1) limits how many top matches are returned.\n\
             • The tool responds with a ready-to-display Markdown block headed “# Profiles”."
        )
        .add_property("name", "string",
            "Full or partial name exactly as given in the user request.")
        .add_property("k", "int",
            "Number of top matches to return (max 5 is sensible).")
        .add_required_property("name")
        .executor(profile_executor)
        .build()?;

    let similar_names_tool = ToolBuilder::new()
        .function_name("get_top_k_similar_names")
        .function_description("Given a name and optionally k (default 5), the tool returns top k similar \
        names of employees to the queried name, based on levenstein distance. Used to lookup names.")
        .add_property("name", "string", "The name that will be used to find similar named employees")
        .add_property("k", "int", "number of names to return")
        .add_required_property("name")
        .executor(similar_names_executor)
        .build()?;

    let agent = AgentBuilder::default()
        .set_model("qwen3:30b")
        .set_ollama_endpoint("http://hivecore.famnit.upr.si")
        .set_ollama_port(6666)
        .set_system_prompt(agent_system_prompt)
        .add_mcp_server(McpServerType::sse(SCRAPER_MCP_URL))
        .add_mcp_server(McpServerType::sse(MEMORY_MCP_URL))
        .set_stopword("<final>")
        .add_tool(staff_profiles_tool)
        .add_tool(similar_names_tool)
        // .set_stop_prompt(stop_prompt)
        .build()
        .await?;



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
    pub async fn ask_staff_expert(&self, #[tool(aggr)] question: StructRequest) -> Result<CallToolResult, rmcp::Error> {
        let mut agent = self.agent.lock().await;
        agent.clear_history();

        let memory_query_args = serde_json::json!({ "query_text": question.question, "top_k": 5 });
        let initial_memory_result = match get_memories(memory_query_args).await {
            Ok(memories) => memories,
            Err(e) => return Ok(CallToolResult::error(vec![Content::text(e.to_string())]))            
        };
        agent.history.push(Message::tool(initial_memory_result, "query_memory"));

        let resp = agent.invoke(question.question).await;

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
            instructions: Some("An agent about employees at the University of Primorska's Faculty of Mathematics, Natural Sciences and Information Technologies (UP FAMNIT)".into()),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            ..Default::default()
        }
    }
}



async fn get_page<T>(url: T) -> Result<String> where T: Into<String> {
    let transport = SseClientTransport::start(SCRAPER_MCP_URL).await?;
    let client_info: rmcp::model::InitializeRequestParam = ClientInfo {
        protocol_version: Default::default(),
        capabilities: ClientCapabilities::default(),
        client_info: Implementation {
            name: "test sse client".to_string(),
            version: "0.0.1".to_string(),
        },
    };
    let client = client_info
        .serve(transport)
        .await
        .inspect_err(|e| {
            println!("client error: {:?}", e);
    })?;

    let tool_result = client
        .clone()
        .call_tool(CallToolRequestParam {
            name: "get_web_page_content".into(),
            arguments: serde_json::json!({"url": url.into()}).as_object().cloned(),
        })
        .await?;

    let mut content = "".into();
    for tool_result_content in tool_result.content {
        content = format!("{}\n{}", content, tool_result_content.as_text().unwrap().text)
    }
    
    Ok(content)
}

async fn get_memories(arguments: serde_json::Value) -> Result<String> {
    let transport = SseClientTransport::start(MEMORY_MCP_URL).await?;
    let client_info: rmcp::model::InitializeRequestParam = ClientInfo {
        protocol_version: Default::default(),
        capabilities: ClientCapabilities::default(),
        client_info: Implementation {
            name: "test sse client".to_string(),
            version: "0.0.1".to_string(),
        },
    };
    let client = client_info
        .serve(transport)
        .await
        .inspect_err(|e| {
            println!("client error: {:?}", e);
    })?;

    let tool_result = client
        .clone()
        .call_tool(CallToolRequestParam {
            name: "query_memory".into(),
            arguments: serde_json::json!(arguments).as_object().cloned(),
        })
        .await?;

    let mut content = "".into();
    for tool_result_content in tool_result.content {
        content = format!("{}\n{}", content, tool_result_content.as_text().unwrap().text)
    }
    
    Ok(content)
}


pub fn staff_html_to_markdown(html: &str) -> HashMap<String, String> {
    let doc     = Html::parse_document(html);
    let row_sel = Selector::parse("#osebje-list tr").unwrap();
    let td_sel  = Selector::parse("td").unwrap();
    let a_sel   = Selector::parse("a").unwrap();

    let mut out = Vec::new();
    let mut names = HashMap::new();
    for row in doc.select(&row_sel) {
        // skip the header row (contains <th> instead of <td>)
        if row.select(&Selector::parse("th").unwrap()).next().is_some() {
            continue;
        }

        let tds: Vec<_> = row.select(&td_sel).collect();
        if tds.len() < 5 { continue; }

        // helpers ----------------------------------------------------------
        let txt = |el: Option<&scraper::ElementRef>| -> String {
            el.map(|e| e.text().collect::<String>().trim().to_owned()).unwrap_or_default()
        };
        let href = |el: Option<&scraper::ElementRef>| -> String {
            el.and_then(|e| e.value().attr("href")).unwrap_or("").to_owned()
        };

        // extract fields ----------------------------------------------------
        let surname_a  = tds[0].select(&a_sel).next();
        let given_a    = tds[1].select(&a_sel).next();
        let email_a    = tds[3].select(&a_sel).next();
        let website_a  = tds[4].select(&a_sel).next();

        let surname     = txt(surname_a.as_ref());
        let given       = txt(given_a.as_ref());
        let _phone       = tds[2].text().collect::<String>().trim().to_owned();
        let _email       = txt(email_a.as_ref());
        let profile_url = href(surname_a.as_ref());
        let _website_url = href(website_a.as_ref());

        // build the markdown bullet ----------------------------------------
        let mut line = format!("- **{} {}**", surname, given);
        // if !email.is_empty()       { line += &format!(" • {}", email); }
        // if !phone.is_empty()       { line += &format!(" • {}", phone); }
        if !profile_url.is_empty() { line += &format!(" • [Profile]({})", profile_url); }
        // if !website_url.is_empty() { line += &format!(" • [Site]({})",    website_url); }
        names.insert(format!("{} {}", given, surname), profile_url);
        out.push(line);
    }

    names
}


