use std::sync::Arc;
use anyhow::Result;
use reagent::{Agent, AgentBuilder, AsyncToolFn, McpServerType, ToolBuilder, ToolExecutionError, Value};

use crate::{profile::StaffProfile, util::{get_page, rank_names, staff_html_to_markdown}, MEMORY_MCP_URL, SCRAPER_MCP_URL};



pub async fn init_staff_agent() -> Result<Agent> {
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

    Ok(agent)

}