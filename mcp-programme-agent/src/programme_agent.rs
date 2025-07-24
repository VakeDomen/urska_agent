use std::{collections::{BTreeMap, HashSet}, sync::Arc};

use reagent::{Agent, AgentBuilder, AsyncToolFn, McpServerType, ToolBuilder, ToolExecutionError, Value};
use anyhow::Result;

use crate::{programme::{Programme, ProgrammeInfo, ProgrammeLevel, ProgrammeSection}, util::{get_page, parse_programme_list_page, rank_names}, MEMORY_MCP_URL, SCRAPER_MCP_URL};

pub async fn init_programme_agent() -> Result<Agent> {
    let agent_system_prompt = r#"
You are **UniProgramme-Agent**, a focused assistant that answers questions about the study programmes offered at *famnit.upr.si*.

────────────────────────────────────────────────────────
1 PROGRAMME LEVELS
• Be aware that programmes are offered at three distinct levels: **Undergraduate**, **Master's**, and **Doctoral**.
• A programme with the same name, like "Computer Science," can exist at multiple levels. Always be precise about the level.

────────────────────────────────────────────────────────
2 AMBIGUITY & CLARIFICATION
• If the user asks for a programme like "Computer Science" without specifying a level, you MUST check for ambiguity. The `get_programme_info` tool will help you with this.
• When you receive an ambiguity message, your next step is to **ask the user a clarifying question**. Do not try to guess the level.

────────────────────────────────────────────────────────
3 ANSWER FORMATTING
• Use Markdown for clear presentation (lists, tables).
• For unknown values, use "—".
• Always specify the programme level in your answer (e.g., "The undergraduate programme in Mathematics...").
• Do not use 'etc.'; provide the full answer.
• If the tool provides a source URL, always include it in your response.
• Your final answer inside the `<final>` tags must be complete and not refer to previous messages.
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
    
                let sections_to_render: Option<HashSet<ProgrammeSection>> = args.get("sections")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                           .filter_map(|s| s.as_str())
                           .filter_map(ProgrammeSection::from_str)
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
            
        let agent = AgentBuilder::plan_and_execute()
            .set_model("qwen3:30b")
            .set_ollama_endpoint("http://hivecore.famnit.upr.si:6666")
            .set_system_prompt(agent_system_prompt.to_string())
            .add_mcp_server(McpServerType::sse(MEMORY_MCP_URL))
            .add_mcp_server(McpServerType::sse(SCRAPER_MCP_URL))
            .add_tool(list_programmes_tool)
            .add_tool(similar_programmes_tool)
            .add_tool(programme_info_tool)
            .build()
            .await?;
        Ok(agent)
}