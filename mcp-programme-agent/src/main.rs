use std::{collections::{BTreeMap, HashMap, HashSet}, fmt, sync::Arc};

use reagent::{init_default_tracing, Agent, AgentBuilder, AsyncToolFn, McpServerType, ToolBuilder, ToolExecutionError, Value};
use reqwest::Url;
use rmcp::{
    model::{CallToolRequestParam, CallToolResult, ClientCapabilities, ClientInfo, Content, Implementation, ServerCapabilities, ServerInfo}, schemars, tool, transport::{SseClientTransport, SseServer}, ServerHandler, ServiceExt
};
use anyhow::Result;
use scraper::{Html, Selector};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::{programme::ProgrammeInfo, util::rank_names};

mod programme;
mod util;

const BIND_ADDRESS: &str = "127.0.0.1:8003";
const BASE_URL: &str = "https://www.famnit.upr.si";

#[derive(Debug, Clone, PartialEq)]
enum ProgrammeLevel {
    Undergraduate,
    Master,
    Doctoral,
}

impl fmt::Display for ProgrammeLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProgrammeLevel::Undergraduate => write!(f, "Undergraduate"),
            ProgrammeLevel::Master => write!(f, "Master's"),
            ProgrammeLevel::Doctoral => write!(f, "Doctoral"),
        }
    }
}

#[derive(Debug, Clone)]
struct Programme {
    name: String,
    url: String,
    level: ProgrammeLevel,
}


#[tokio::main]
async fn main() -> Result<()> {
    init_default_tracing();
    let agent_system_prompt = r#"
You are **UniProgramme-Agent**, a focused assistant that answers questions about the study programmes offered at *famnit.upr.si*. You build up a factual **long-term memory** about these programmes.

────────────────────────────────────────────────────────
1 PROGRAMME LEVELS
• Be aware that programmes are offered at three distinct levels: **Undergraduate**, **Master's**, and **Doctoral**.
• A programme with the same name, like "Computer Science," can exist at multiple levels. Always be precise about the level.

────────────────────────────────────────────────────────
2 PLANNING & REFLECTION  
• **Immediately after reading the user’s request, draft a short, numbered plan** that lists the steps you intend to take.
• After every tool call, **reflect** on your progress and update the plan if necessary.

────────────────────────────────────────────────────────
3 MEMORY-FIRST, BUT VERIFY
• **Always start** with `query_memory`. Use retrieved memories to inform your plan.
• **Crucial Principle:** Your memory is a helpful starting point, but it can be incomplete. The tools are the source of truth.
• If the user asks for a list or a count of items (e.g., "list all courses", "how many requirements"), and you find a relevant memory, you **must still use a tool to fetch the complete, definitive list** before answering.

────────────────────────────────────────────────────────
4 AMBIGUITY & CLARIFICATION
• If the user asks for a programme like "Computer Science" without specifying a level, you MUST check for ambiguity.
• The `get_programme_info` tool will help you by returning a clarification message if multiple levels are found.
• When you receive such a message, your next step is to **ask the user a clarifying question**. Do not try to guess the level.

────────────────────────────────────────────────────────
5 TOOLS – WHEN & HOW

→ **query_memory** – Always the first call.

→ **list_all_programmes** – Use when the user wants a general list of programmes.

→ **get_similar_programme_names** - Use to find programme names when the user's query is misspelled or a partial match.

→ **get_programme_info** – Use to get definitive information about a programme. This tool always fetches live data from the website to ensure the information is up-to-date.
  • **BE EFFICIENT:** Use the `sections` parameter to request **only the information you need.**
  • Example (User asks for admission requirements): `{ "name": "...", "level": "...", "sections": ["admission_requirements"] }`
  • Example (User asks for a list of courses): `{ "name": "...", "level": "...", "sections": ["course_tables"] }`
  • **Valid section names are**: `general_info`, `coordinators`, `about`, `goals`, `course_structure`, `field_work`, `course_tables`, `admission_requirements`, `transfer_criteria`, `advancement_requirements`, `completion_requirements`, `competencies`, `employment_opportunities`.

→ **store_memory** – Call this to save new or corrected information.
  • **IMPORTANT: Before storing, check if the fact already exists in memory. DO NOT add duplicate information.**
  • If a tool call revealed that a memory was incomplete or incorrect, use this to save the updated fact.
  • Call **before** sending the final answer.

────────────────────────────────────────────────────────
6 WORKFLOW (after initial memory check)

1.  Produce a plan.
2.  If the user asks for a general list of programmes, use `list_all_programmes`.
3.  If the user asks for specific details about a programme (especially a list or a count of items):
    a. Determine the programme name, level, and the specific sections needed.
    b. **Even if memory has a partial answer, call `get_programme_info` with the correct `sections` to get the complete and authoritative information.**
    c. If the tool returns an ambiguity message, update your plan to ask the user for clarification.
    d. Once you have the definitive information from the tool, compare it to your memory. Formulate your final answer using the **complete information from the tool output.**
4.  **Before storing new facts with `store_memory`, review your initial `query_memory` results to ensure you are not adding duplicate data.** If your tool call corrected an incomplete memory, you should store the new, complete fact.
5.  When all information is gathered and stored, wrap the final answer in `<final> … </final>`.

────────────────────────────────────────────────────────
7 ANSWER FORMATTING
• Use Markdown for clear presentation (lists, tables).
• Provide full lists when listing items, never just partial information.
• For unknown values, use "—".
• Always specify the programme level in your answer (e.g., "The undergraduate programme in Mathematics...").
• Do not use 'etc.', but write the whole answer.
• If you refer the user to an external resource, or if the tool provides a source URL, always include it in your response.
• In the <final>message</final> write the whole answer and avoid referencing the user to previous messages as they don't see anything outside <final> tags.
"#;

    let programme_sources = vec![
        ("https://www.famnit.upr.si/en/education/undergraduate", ProgrammeLevel::Undergraduate),
        ("https://www.famnit.upr.si/en/education/master", ProgrammeLevel::Master),
        ("https://www.famnit.upr.si/en/education/doctoral", ProgrammeLevel::Doctoral),
    ];

    let mut all_programmes: Vec<Programme> = Vec::new();
    for (url, level) in programme_sources {
        match get_page(url).await {
            Ok(html) => {
                let mut parsed_programmes = parse_programme_list_page(&html, level);
                all_programmes.append(&mut parsed_programmes);
            }
            Err(e) => eprintln!("Could not fetch or parse page for URL {}: {}", url, e),
        }
    }
    
    let all_programmes_clone_for_list = all_programmes.clone();
    let list_programmes_executor: AsyncToolFn = Arc::new(move |args: Value| {
        let programmes_list = all_programmes_clone_for_list.clone();
        Box::pin(async move {
            let level_filter = args.get("level").and_then(|v| v.as_str());
            let target_level = match level_filter {
                Some("undergraduate") => Some(ProgrammeLevel::Undergraduate),
                Some("master") => Some(ProgrammeLevel::Master),
                Some("doctoral") => Some(ProgrammeLevel::Doctoral),
                _ => None,
            };

            let mut result_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for prog in programmes_list {
                if target_level.is_none() || Some(prog.level.clone()) == target_level {
                    result_map.entry(prog.level.to_string()).or_default().push(prog.name);
                }
            }

            if result_map.is_empty() { return Ok("No programmes found for the specified level.".to_string()); }

            let mut md = String::new();
            for (level, progs) in result_map {
                md.push_str(&format!("\n### {}\n", level));
                for name in progs { md.push_str(&format!("- {}\n", name)); }
            }
            Ok(md)
        })
    });
    
    let list_programmes_tool = ToolBuilder::new()
        .function_name("list_all_programmes")
        .function_description("Lists the names of available study programmes. Can be filtered by study level to list only undergraduate, master's, or doctoral programmes.")
        .add_property("level", "string", "Optional level to filter by. Accepted values: 'undergraduate', 'master', 'doctoral'.")
        .executor(list_programmes_executor)
        .build()?;

    let all_programmes_clone_for_similar = all_programmes.clone();
    let similar_programmes_executor: AsyncToolFn = Arc::new(move |args: Value| {
        let programmes_list = all_programmes_clone_for_similar.clone();
        Box::pin(async move {
            let query_name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| ToolExecutionError::ArgumentParsingError("Missing 'name' argument".into()))?;
            let k = args.get("k").and_then(|v| v.as_i64()).unwrap_or(5);
            let level_filter = args.get("level").and_then(|v| v.as_str());

            let target_level = match level_filter {
                Some("undergraduate") => Some(ProgrammeLevel::Undergraduate),
                Some("master") => Some(ProgrammeLevel::Master),
                Some("doctoral") => Some(ProgrammeLevel::Doctoral),
                _ => None,
            };

            let names_to_rank: Vec<String> = programmes_list
                .into_iter()
                .filter(|p| target_level.is_none() || Some(p.level.clone()) == target_level)
                .map(|p| p.name)
                .collect();

            if names_to_rank.is_empty() { return Ok("No programmes found for the specified level.".to_string()); }

            let ranked_names = rank_names(names_to_rank, query_name);
            let top_k = ranked_names.into_iter().take(k as usize).collect::<Vec<String>>();
            Ok(top_k.join(" \n - "))
        })
    });

    let similar_programmes_tool = ToolBuilder::new()
        .function_name("get_similar_programme_names")
        .function_description("Given a programme name, returns top k similar names. Can be filtered by study level.")
        .add_property("name", "string", "The name to find similar programmes for.").add_required_property("name")
        .add_property("k", "int", "Number of names to return (default 5).")
        .add_property("level", "string", "Optional level to filter by: 'undergraduate', 'master', or 'doctoral'.")
        .executor(similar_programmes_executor)
        .build()?;

    let all_programmes_clone_for_info = all_programmes.clone();
    let programme_info_executor: AsyncToolFn = Arc::new(move |args: Value| {
        let programmes_list = all_programmes_clone_for_info.clone();
        Box::pin(async move {
            let query_name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| ToolExecutionError::ArgumentParsingError("Missing 'name' argument".into()))?;
            let level_filter = args.get("level").and_then(|v| v.as_str());

            let all_names: Vec<String> = programmes_list.iter().map(|p| p.name.clone()).collect();
            let top_ranked_names = rank_names(all_names, query_name);
            let best_match_name = match top_ranked_names.first() {
                Some(name) => name,
                None => return Ok(format!("No programme found matching the name '{}'.", query_name)),
            };

            let mut potential_matches: Vec<Programme> = programmes_list
                .into_iter()
                .filter(|p| p.name.eq_ignore_ascii_case(best_match_name))
                .collect();

            if let Some(level_str) = level_filter {
                let target_level = match level_str.to_lowercase().as_str() {
                    "undergraduate" => Some(ProgrammeLevel::Undergraduate),
                    "master" => Some(ProgrammeLevel::Master),
                    "doctoral" => Some(ProgrammeLevel::Doctoral),
                    _ => None,
                };
                if let Some(level) = target_level {
                    potential_matches.retain(|p| p.level == level);
                }
            }
            
            if potential_matches.len() > 1 {
                let levels: Vec<String> = potential_matches.iter().map(|p| p.level.to_string()).collect();
                return Ok(format!("Found '{}' at multiple levels: {}. Please specify which one you are interested in.", best_match_name, levels.join(", ")));
            }

            let target_programme = match potential_matches.first() {
                Some(p) => p,
                None => return Ok(format!("No programme found for '{}' at the specified level.", best_match_name)),
            };

            let sections_to_render: Option<HashSet<programme::ProgrammeSection>> = args.get("sections")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                       .filter_map(|s| s.as_str())
                       .filter_map(programme::ProgrammeSection::from_str)
                       .collect()
                });
            
            let mut result = String::new();
            match get_page(&target_programme.url).await {
                Ok(html) => {
                    let info = ProgrammeInfo::from(html);
                    result.push_str(&info.to_markdown(sections_to_render.as_ref()));
                    // Append the source URL to the result
                    result.push_str(&format!("\n\n---\n*Source: [{}]({})*", target_programme.url, target_programme.url));
                }
                Err(_) => {
                    result = format!("Could not retrieve information for '{}'.", target_programme.name);
                }
            }
            Ok(result)
        })
    });

    let programme_info_tool = ToolBuilder::new()
        .function_name("get_programme_info")
        .function_description(
            "Return detailed programme information (ECTS, duration, etc.). If a programme with the same name exists at multiple levels, you must use the 'level' parameter to disambiguate. Use the 'sections' parameter to be efficient and request only the information you need."
        )
        .add_property("name", "string", "Full or partial name of the study programme.").add_required_property("name")
        .add_property("level", "string", "Optional study level to filter by: 'undergraduate', 'master', or 'doctoral'.")
        .add_property("sections", "array", 
            "Optional. A list of specific sections to return. Valid sections: 'general_info', 'coordinators', 'about', 'goals', 'course_structure', 'field_work', 'course_tables', 'admission_requirements', 'transfer_criteria', 'advancement_requirements', 'completion_requirements', 'competencies', 'employment_opportunities'."
        )
        .executor(programme_info_executor)
        .build()?;
        
    let agent = AgentBuilder::default()
        .set_model("qwen3:30b")
        .set_ollama_endpoint("http://hivecore.famnit.upr.si")
        .set_ollama_port(6666)
        .set_system_prompt(agent_system_prompt.to_string())
        .add_mcp_server(McpServerType::sse("http://localhost:8000/sse"))
        .add_mcp_server(McpServerType::sse("http://localhost:8002/sse"))
        .set_stopword("<final>")
        .add_tool(list_programmes_tool)
        .add_tool(similar_programmes_tool)
        .add_tool(programme_info_tool)
        .build()
        .await?;

    let ct = SseServer::serve(BIND_ADDRESS.parse()?)
        .await?
        .with_service(move || Service::new(agent.clone()));

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

    #[tool(description = "Ask the agent")]
    pub async fn ask(&self, #[tool(aggr)] question: StructRequest) -> Result<CallToolResult, rmcp::Error> {
        let mut agent = self.agent.lock().await;
        agent.clear_history();
        let resp = agent.invoke(question.question).await;
        let _memory_resp = agent.invoke("Is there any memory you would like to store?").await;
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

async fn get_page<T>(url: T) -> Result<String> where T: Into<String> {
    let transport = SseClientTransport::start("http://localhost:8000/sse").await?;
    let client_info: rmcp::model::InitializeRequestParam = ClientInfo {
        protocol_version: Default::default(),
        capabilities: ClientCapabilities::default(),
        client_info: Implementation {
            name: "test sse client".to_string(),
            version: "0.0.1".to_string(),
        },
    };
    let client = client_info.serve(transport).await.inspect_err(|e| { println!("client error: {:?}", e); })?;
    let tool_result = client.clone().call_tool(CallToolRequestParam {
            name: "get_web_page_content".into(),
            arguments: serde_json::json!({"url": url.into()}).as_object().cloned(),
        }).await?;
    let mut content = "".into();
    for tool_result_content in tool_result.content {
        content = format!("{}\n{}", content, tool_result_content.as_text().unwrap().text)
    }
    Ok(content)
}

fn parse_programme_list_page(html: &str, level: ProgrammeLevel) -> Vec<Programme> {
    let doc = Html::parse_document(html);
    let selector = Selector::parse("div.content ul li a").unwrap();
    let base_url = Url::parse(BASE_URL).expect("Failed to parse base URL");
    let mut programmes = Vec::new();

    for element in doc.select(&selector) {
        let raw_name = element.text().collect::<String>();
        let name = raw_name.split('(').next().unwrap_or("").trim().to_string();

        if let Some(href) = element.value().attr("href") {
            if !name.is_empty() && !href.starts_with("javascript:") {
                if let Ok(full_url) = base_url.join(href) {
                    programmes.push(Programme {
                        name: name.clone(),
                        url: full_url.to_string(),
                        level: level.clone(),
                    });
                }
            }
        }
    }
    programmes
}
