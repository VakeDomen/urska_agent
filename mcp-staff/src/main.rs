use std::{collections::HashMap, sync::Arc};

use reagent::{init_default_tracing, Agent, Message};
use rmcp::{
    handler::server::tool::{Parameters, ToolRouter}, model::{CallToolResult, Content, Meta, ProgressNotificationParam, ServerCapabilities, ServerInfo}, schemars, tool, tool_handler, tool_router, transport::{streamable_http_server::session::local::LocalSessionManager, StreamableHttpService}, Peer, RoleServer, ServerHandler
};
use anyhow::Result;
use serde::Deserialize;
use tokio::sync::{mpsc, Mutex, OnceCell};

use crate::{profile::StaffProfile, util::{get_page, rank_names, staff_html_to_markdown}};


mod profile;
mod util;

const BIND_ADDRESS: &str = "127.0.0.1:8001";
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
    
    println!("- Staff MCP running at {}", BIND_ADDRESS);
    println!("(Press Ctrl+C to terminate immediately)");

    axum::serve(tcp_listener, router).await?;


    Ok(())
}



#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SimilarNamesRequest {
    /// The name that will be used to find similar named employees.
    pub name: String,
    /// Number of names to return (default is 5).
    pub k: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StaffProfilesRequest {
    /// Full or partial name to find staff profiles for.
    pub name: String,
    /// Number of top matches to return (default is 1).
    pub k: Option<i64>,
}

// --- Service Implementation ---

#[derive(Debug, Clone)]
struct Service {
    tool_router: ToolRouter<Service>,
    // Use OnceCell for lazy async initialization of the staff list.
    all_staff: Arc<OnceCell<HashMap<String, String>>>,
}

#[tool_router]
impl Service {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            all_staff: Arc::new(OnceCell::new()),
        }
    }

    /// Helper function to scrape the staff list exactly once.
    async fn get_or_init_staff_list(&self) -> Result<&HashMap<String, String>> {
        self.all_staff.get_or_try_init(|| async {
            let staff_list_html = get_page("https://www.famnit.upr.si/en/about-faculty/staff/").await?;
            let staff_map = staff_html_to_markdown(&staff_list_html);
            Ok::<_, anyhow::Error>(staff_map)
        }).await
    }

    #[tool(
        name = "get_similar_staff_names",
        description = "Given a name and optionally k (default 5), the tool returns top k similar names of employees to the queried name, based on levenstein distance. Used to lookup names."
    )]
    pub async fn get_similar_staff_names(
        &self,
        Parameters(request): Parameters<SimilarNamesRequest>,
        _client: Peer<RoleServer>,
        _meta: Meta,
    ) -> Result<CallToolResult, rmcp::Error> {
        let Ok(staff_map) = self.get_or_init_staff_list().await else {
            return Ok(CallToolResult::error(vec![Content::text(
                "Could not retrieve inital staff list. This is an error."
            )]))
        };

        let k = request.k.unwrap_or(5);
        let all_names: Vec<String> = staff_map.keys().cloned().collect();

        if all_names.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("Could not retrieve staff list.")]));
        }

        let ranked_names = rank_names(all_names, &request.name);
        let top_k = ranked_names.into_iter().take(k as usize).collect::<Vec<String>>();
        let response = top_k.join(" \n");

        Ok(CallToolResult::success(vec![Content::text(response)]))
    }

    #[tool(
        name = "get_staff_profiles",
        description = "Return detailed staff profile(s) in Markdown. Use when the user asks for full information (office, phone, courses…). Pass the query string as 'name'. Optional 'k' (default 1) limits how many top matches are returned."
    )]
    pub async fn get_staff_profiles(
        &self,
        Parameters(request): Parameters<StaffProfilesRequest>,
        _client: Peer<RoleServer>,
        _meta: Meta,
    ) -> Result<CallToolResult, rmcp::Error> {
        
        let Ok(staff_map) = self.get_or_init_staff_list().await else {
            return Ok(CallToolResult::error(vec![Content::text(
                "Could not retrieve inital staff list. This is an error."
            )]))
        };

        let k = request.k.unwrap_or(1);
        let all_names: Vec<String> = staff_map.keys().cloned().collect();

        if all_names.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text("Could not retrieve staff list.")]));
        }

        let top_names = rank_names(all_names, &request.name)
            .into_iter()
            .take(k as usize)
            .collect::<Vec<String>>();

        let mut result = String::from("# Profiles\n");

        for name in top_names {
            if let Some(profile_url) = staff_map.get(&name) {
                match get_page(profile_url).await {
                    Ok(profile_page_html) => {
                        let profile = StaffProfile::from(profile_page_html);
                        result.push_str("\n---\n\n");
                        result.push_str(&profile.to_string());
                    }
                    Err(e) => {
                        // Log the error but continue, so one failed profile doesn't kill the whole request.
                        eprintln!("Failed to fetch profile for {}: {}", name, e);
                        result.push_str(&format!("\n---\n\nCould not retrieve profile for {}.\n", name));
                    }
                }
            }
        }

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }
}

#[tool_handler]
impl ServerHandler for Service {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("A set of tools for querying information about university staff at UP FAMNIT.".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}