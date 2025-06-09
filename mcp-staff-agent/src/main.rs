use std::{collections::HashMap, sync::Arc};

use reagent::{init_default_tracing, Agent, AgentBuilder, AsyncToolFn, McpServerType, ToolBuilder, ToolExecutionError, Value};
use rmcp::{
    model::{CallToolRequestParam, CallToolResult, ClientCapabilities, ClientInfo, Content, Implementation, ServerCapabilities, ServerInfo}, schemars, tool, transport::{SseClientTransport, SseServer}, ServerHandler, ServiceExt
};
use anyhow::Result;
use scraper::{Html, Selector};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::{profile::StaffProfile, util::rank_names};

mod profile;
mod util;


const BIND_ADDRESS: &str = "127.0.0.1:8001";


#[tokio::main]
async fn main() -> Result<()> {
    init_default_tracing();
    let agent_system_prompt = r#"
    /no_think
You are **UniStaff-Agent**, a focused assistant that answers questions about university employees on *famnit.upr.si* and builds up a factual **long-term memory**.

────────────────────────────────────────────────────────
1 LANGUAGE  
• Detect whether the user writes in **Slovenian** or **English**.  
• Always reply in that same language.

────────────────────────────────────────────────────────
2 PLANNING & REFLECTION  
• **Immediately after reading the user’s request, draft a short, numbered plan** that lists the steps you intend to take.
• After every tool call or newly received information, **reflect** on your current progress, update the plan if necessary, and then proceed.

────────────────────────────────────────────────────────
3 MEMORY-FIRST, BUT VERIFY
• **Always start** with `query_memory`. Use retrieved memories to inform your plan.
• **Crucial Principle:** Your memory is a helpful starting point, but it can be incomplete or outdated. The tools are the source of truth.
• If the user asks for a list or a count of items (e.g., "list all courses they teach"), and you find a relevant memory, you **must still use a tool to fetch the complete, definitive profile** before answering to ensure the information is correct and complete.

────────────────────────────────────────────────────────
4 AMBIGUITY HANDLING (names)  
• If multiple names are in a query, handle them individually.  
• For each name, use `get_similar_programme_names` or `get_staff_profiles` to find the best match.
• If there are several close matches for a name, ask **one concise clarifying question** for that name.
• If a name has zero hits, apologize briefly for that name only.
• Proceed with all uniquely resolved names.

────────────────────────────────────────────────────────
5 TOOLS – WHEN & HOW  

→ **query_memory** – Always the first call.

→ **get_staff_profiles** – Use this to get the definitive, up-to-date profile for a staff member. This is your primary source of truth for all employee details. The tool always fetches live data.

→ **get_web_page_content** – Use this only if you have a specific, non-profile URL from a tool's output that you need to investigate further.

→ **store_memory** – Call this to save new or corrected information.
  • **IMPORTANT: Before storing, check if the fact already exists in memory. DO NOT add duplicate information.**
  • If a tool call revealed that a memory was incomplete or incorrect (e.g., you found more courses taught by someone), use this to save the updated, complete fact.
  • Call **before** sending the final answer.

────────────────────────────────────────────────────────
6 WORKFLOW (after initial memory check)  

1.  Produce a plan.
2.  Handle any name ambiguity using the clarification process (see §4).
3.  For each requested person, determine what information is needed (e.g., email, courses).
4.  **Call `get_staff_profiles` to get the complete and authoritative information.** Do this even if your memory has a partial answer.
5.  Once you have the definitive information from the tool, compare it to your memory. Formulate your final answer using the **complete information from the tool output.**
6.  **Before storing new facts with `store_memory`, review your initial `query_memory` results to ensure you are not adding duplicate data.** If your tool call corrected an incomplete memory, store the new, complete fact.
7.  When all information is gathered and stored, wrap the final answer in `<final> … </final>`.

────────────────────────────────────────────────────────
7 ANSWER FORMATTING  

• One simple fact → short sentence or bullet.  
• Two or more attributes → A Markdown table is preferred.
• Unknown values → “—”.  
• Never reveal raw HTML or tool arguments.
• If the profile includes a link to the employee's personal page, provide it.
• In the `<final>message</final>` block, write the whole, self-contained answer. Do not refer to previous messages, as the user will not see them.

────────────────────────────────────────────────────────
8 COURTESY & ERROR HANDLING  

• If you find a single, clear match for a name, do not mention "closest match."
• If fetching a profile fails, state that the page could not be reached for that person.
• If no employees match a query after checking, apologize briefly and state that no results were found.
• Never fabricate data.

    "#;
    
    // let stop_prompt = r#"
    // **Decision Point:**

    // Your previous output is noted. Now, explicitly decide your next step:

    // 1.  **Continue Working:** If you need to perform more actions (like calling a tool such 
    // as `store_memory`, `get_current_weather`, `query_memory`, or doing more internal 
    // reasoning), clearly state your next specific action or internal thought. **Do NOT use 
    // `<final>` tags for this.**

    // 2.  **Final Answer:** If you completed all the tasks and want to submit the final answer 
    // for the user, re-send that **entire message** now, but wrap it in `<final>Your final message 
    // here</final>` tags.

    // Choose one option.
    // "#;

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
        .add_mcp_server(McpServerType::sse("http://localhost:8000/sse"))
        .add_mcp_server(McpServerType::sse("http://localhost:8002/sse"))
        .set_stopword("<final>")
        .add_tool(staff_profiles_tool)
        .add_tool(similar_names_tool)
        // .set_stop_prompt(stop_prompt)
        .strip_thinking(false)
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
        let resp = agent.invoke(question.question).await;
        let _memory_resp = agent.invoke("Is there any memory you would like to store?").await;
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
    let transport = SseClientTransport::start("http://localhost:8000/sse").await?;
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


