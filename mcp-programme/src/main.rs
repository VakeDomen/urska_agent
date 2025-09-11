use std::{collections::{BTreeMap, HashSet}, sync::Arc};

use anyhow::Result;
use reagent::{init_default_tracing, util::invocations::invoke_without_tools, Agent, Message};
use rmcp::{handler::server::tool::{Parameters, ToolRouter}, model::{CallToolResult, Content, Meta, ProgressNotificationParam, ServerCapabilities, ServerInfo}, schemars, tool, tool_handler, tool_router, transport::{streamable_http_server::session::local::LocalSessionManager, StreamableHttpService}, Peer, RoleServer, ServerHandler};
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex, OnceCell};

use crate::{programme::{Programme, ProgrammeInfo, ProgrammeLevel, ProgrammeSection}, util::{get_page, parse_programme_list_page, rank_names}};



mod programme;
mod util;

const BIND_ADDRESS: &str = "127.0.0.1:8003";
const BASE_URL: &str = "https://www.famnit.upr.si";
const MEMORY_MCP_URL: &str = "http://localhost:8002/mcp";
const SCRAPER_MCP_URL: &str = "http://localhost:8000/sse";


#[tokio::main]
async fn main() -> Result<()> {
    init_default_tracing();


    let service = StreamableHttpService::new(
        move || Ok(Service::new()),
        LocalSessionManager::default().into(),
        Default::default(),
    );

    let router = axum::Router::new().nest_service("/mcp", service);
    let tcp_listener = tokio::net::TcpListener::bind(BIND_ADDRESS).await?;
    
    println!("- Programme MCP running at {}", BIND_ADDRESS);
    println!("(Press Ctrl+C to terminate immediately)");

    axum::serve(tcp_listener, router).await?;


    Ok(())
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListProgrammesRequest {
    /// Optional level to filter by. Accepted values: 'undergraduate', 'master', 'doctoral' or 'any'.
    pub level: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SimilarProgrammesRequest {
    /// The name to find similar programmes for.
    pub name: String,
    /// Number of names to return (default 5).
    pub k: Option<i32>,
    /// Optional level to filter by: 'undergraduate', 'master', 'doctoral' or 'any'.
    pub level: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ProgrammeInfoRequest {
    /// Full or partial name of the study programme.
    pub name: String,
    /// Optional study level to filter by: 'undergraduate', 'master', 'doctoral' or 'any'.
    pub level: Option<String>,
    // Optional. A list of specific sections to return. Valid sections: 'general_info', 'coordinators', 'about', 'goals', 'course_structure', 'field_work', 'course_tables', 'admission_requirements', 'transfer_criteria', 'advancement_requirements', 'completion_requirements', 'competencies', 'employment_opportunities'.
    // pub sections: Option<Vec<String>>,
}


#[derive(Debug, Clone)]
struct Service {
    tool_router: ToolRouter<Service>,
    all_programmes: Arc<OnceCell<Vec<Programme>>>,
}

#[tool_router]
impl Service {
    pub fn new() -> Self {
        Self { 
            tool_router: Self::tool_router(),
            all_programmes: Arc::new(OnceCell::new()),
        }
    }

    async fn get_or_init_programmes(&self) -> Result<&Vec<Programme>> {
        self.all_programmes.get_or_try_init(|| async {
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
            Ok::<_, anyhow::Error>(all_programmes)
        }).await
    }

    #[tool(
        name = "list_all_programmes",
        description = "Lists the names of available study programmes. Can be filtered by study level to list only undergraduate, master's, or doctoral programmes."
    )]
    pub async fn list_all_programmes(
        &self, 
        Parameters(request): Parameters<ListProgrammesRequest>,
        _client: Peer<RoleServer>,
        _meta: Meta
    ) -> Result<CallToolResult, rmcp::Error> {
        let Ok(programmes) = self.get_or_init_programmes().await else {
            return Ok(CallToolResult::error(vec![Content::text("
                Can't find any programmes. This is an error.
            ")]))
        };

        let target_level = match request.level.as_deref() {
            Some("undergraduate") => Some(ProgrammeLevel::Undergraduate),
            Some("master") => Some(ProgrammeLevel::Master),
            Some("doctoral") => Some(ProgrammeLevel::Doctoral),
            _ => None,
        };

        let mut result_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for prog in programmes.iter() {
            if target_level.is_none() || Some(prog.level.clone()) == target_level {
                result_map.entry(prog.level.to_string()).or_default().push(prog.name.clone());
            }
        }

        if result_map.is_empty() { 
            return Ok(CallToolResult::success(vec![Content::text("No programmes found for the specified level.")]));
        }

        let mut md = String::new();
        for (level, progs) in result_map {
            md.push_str(&format!("\n### {}\n", level));
            for name in progs { md.push_str(&format!("- {}\n", name)); }
        }
        
        Ok(CallToolResult::success(vec![Content::text(md)]))
    }

    // #[tool(
    //     name = "get_similar_programme_names",
    //     description = "Given a programme name, returns top k similar names, including their study level. Can be filtered by study level."
    // )]
    // pub async fn get_similar_programmes(
    //     &self,
    //     Parameters(request): Parameters<SimilarProgrammesRequest>,
    //     _client: Peer<RoleServer>,
    //     _meta: Meta,
    // ) -> Result<CallToolResult, rmcp::Error> {
    //     let Ok(programmes) = self.get_or_init_programmes().await else {
    //         return Ok(CallToolResult::error(vec![Content::text("
    //             Can't find any programmes. This is an error.
    //         ")]))
    //     };

    //     let k = request.k.unwrap_or(5);

    //     let target_level = match request.level.as_deref() {
    //         Some("undergraduate") => Some(ProgrammeLevel::Undergraduate),
    //         Some("master") => Some(ProgrammeLevel::Master),
    //         Some("doctoral") => Some(ProgrammeLevel::Doctoral),
    //         _ => None,
    //     };

    //     let filtered_programmes: Vec<&Programme> = programmes
    //         .iter()
    //         .filter(|p| target_level.is_none() || Some(p.level.clone()) == target_level)
    //         .collect();

    //     if filtered_programmes.is_empty() {
    //         return Ok(CallToolResult::success(vec![Content::text(
    //             "No programmes found for the specified level.",
    //         )]));
    //     }

    //     let names_to_rank: Vec<String> = filtered_programmes
    //         .iter()
    //         .map(|p| format!("- {} ({})", p.name, p.level))
    //         .collect();

    //     // Rank the combined strings.
    //     let ranked_names = rank_names(names_to_rank, &request.name);
        
    //     // The ranked list already contains the formatted strings, so we just take the top K and join them.
    //     let top_k = ranked_names
    //         .into_iter()
    //         .take(k as usize)
    //         .collect::<Vec<String>>();

    //     let response = top_k.join(" \n");

    //     Ok(CallToolResult::success(vec![Content::text(response)]))
    // }


    #[tool(
        name = "get_programme_info",
        description = "Return detailed programme information (ECTS, duration, etc.). If a programme with the same name exists at multiple levels, you must use the 'level' parameter to disambiguate."
    )]
    pub async fn get_programme_info(
        &self,
        Parameters(request): Parameters<ProgrammeInfoRequest>,
        _client: Peer<RoleServer>,
        _meta: Meta
    ) -> Result<CallToolResult, rmcp::Error> {
        let Ok(programmes) = self.get_or_init_programmes().await else {
            return Ok(CallToolResult::error(vec![Content::text("
                Can't find any programmes. This is an error.
            ")]))
        };

        let all_names: Vec<String> = programmes.iter().map(|p| p.name.clone()).collect();
        let top_ranked_names = rank_names(all_names, &request.name);
        let best_match_name = match top_ranked_names.first() {
            Some(name) => name,
            None => return Ok(CallToolResult::success(vec![Content::text(format!("No programme found matching the name '{}'.", request.name))])),
        };

        let mut potential_matches: Vec<Programme> = programmes
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(best_match_name))
            .cloned()
            .collect();

        if let Some(level_str) = request.level {
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
            let response = format!("Found '{}' at multiple levels: {}. Please specify which one you are interested in.", best_match_name, levels.join(", "));
            return Ok(CallToolResult::success(vec![Content::text(response)]));
        }

        let target_programme = match potential_matches.first() {
            Some(p) => p,
            None => return Ok(CallToolResult::success(vec![Content::text(format!("No programme found for '{}' at the specified level.", best_match_name))])),
        };

        let sections_to_render: Option<HashSet<ProgrammeSection>> = {
            let section_names = vec![
                "general_info", 
                "coordinators", 
                "about", 
                "goals", 
                "course_structure", 
                "field_work", 
                "course_tables", 
                "admission_requirements", 
                "transfer_criteria", 
                "advancement_requirements", 
                "completion_requirements", 
                "competencies", 
                "employment_opportunities"
            ];
            let set: HashSet<ProgrammeSection> = section_names
                .into_iter()
                .filter_map(|s| ProgrammeSection::from_str(s))
                .collect();
            Some(set)
        };
        
        let mut result = String::new();
        match get_page(&target_programme.url).await {
            Ok(html) => {
                let info = ProgrammeInfo::from(html);
                result.push_str(&info.to_markdown(sections_to_render.as_ref()));
                result.push_str(&format!("\n\n---\n*Source: [{}]({})*", target_programme.url, target_programme.url));
            }
            Err(_) => {
                result = format!("Could not retrieve information for '{}'.", target_programme.name);
            }
        }

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }
}

#[tool_handler]
impl ServerHandler for Service {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("A set of tools for querying information about study programmes at UP FAMNIT.".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}