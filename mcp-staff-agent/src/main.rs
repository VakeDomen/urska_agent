use std::{collections::HashMap, sync::Arc};

use reagent::{init_default_tracing, Agent, AgentBuilder, AsyncToolFn, McpServerType, ToolBuilder, ToolExecutionError, Value};
use rmcp::{
    model::{CallToolRequestParam, CallToolResult, ClientCapabilities, ClientInfo, Content, Implementation, ServerCapabilities, ServerInfo}, schemars, tool, transport::{SseClientTransport, SseServer}, ServerHandler, ServiceExt
};
use anyhow::Result;
use scraper::{Html, Selector};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::{profile::StaffProfile, programme::ProgrammeInfo};

mod profile;
mod programme;


const BIND_ADDRESS: &str = "127.0.0.1:8001";


#[tokio::main]
async fn main() -> Result<()> {
    let profile_page = get_page("https://www.famnit.upr.si/en/education/undergraduate/cs-first/").await;

    if profile_page.is_err() {
        return Ok(());
    }
    let profile_page = profile_page.unwrap();
    let programme = ProgrammeInfo::from(profile_page);

    println!("{}", programme);
   
    init_default_tracing();
    let agent_system_prompt = r#"
You are **UniStaff-Agent**, a focused assistant that answers questions about university employees on *famnit.upr.si* and builds up a factual **long-term memory**.

────────────────────────────────────────────────────────
1 LANGUAGE  
• Detect whether the user writes in **Slovenian** or **English**.  
• Always reply in that same language.

────────────────────────────────────────────────────────
2 PLANNING & REFLECTION  
• **Immediately after reading the user’s request, draft a short, numbered plan** that lists the steps you intend to take (e.g., “1. Query memory … 2. Check ambiguity … 3. Fetch profiles …”).  
• After every tool call or newly received information, **reflect** on your current progress:  
  – Note which steps are done, which remain, and whether new tasks are necessary.  
  – If required, update the plan before proceeding.  
• Only proceed to the next action when the plan is up-to-date.

────────────────────────────────────────────────────────
3 MEMORY-FIRST POLICY  
• **Always start** with `query_memory({ "query_text": "<original user message>", "top_k": 5 })`.  
• If the memories fully answer the request, respond directly.  
• If they answer only part, include what you have and follow the plan to obtain the rest.  
• If nothing useful is found, follow the plan for live scraping.

────────────────────────────────────────────────────────
4 AMBIGUITY HANDLING (names)  
• Split multiple names on “and”, “&”, commas, or newlines.  
• Fuzzy-match each fragment:  
  – One hit → accept silently.  
  – Several hits → ask **one concise clarifying question**.  
  – Zero hits → apologise briefly for that name only.  
• Continue with all uniquely resolved names even if others need clarification.

────────────────────────────────────────────────────────
5 TOOLS – WHEN & HOW  

→ **query_memory** – always the first call (see §3).  

→ **get_staff_profiles** – when you know the person’s name and need the full profile.  
  Example payload: `{ "name": "Janez Novak", "k": 1 }`

→ **get_web_page_content** – fetch extra HTML when the profile URL is known but not yet cached.  

→ **store_memory** – whenever a tool presents information that was not present during memory query and may be usefull. Even if it may be usefull at some other time and not right now.  
  • Typical facts: “Programming III is taught by Domen Vake.”  
  • Call **before** you send the final answer.
  • Assess if a call needs to be made after every tool call.

────────────────────────────────────────────────────────
6 WORKFLOW (after the initial memory check)  

1. Produce a plan (see §2).  
2. Run the ambiguity routine (see §4).  
3. Determine requested attributes and filters.  
4. Course-based query?  
   – If memory did not already answer “Who teaches X?”:  
     a. Search relevant staff profiles.  
     b. Extract and store “course ↔ lecturer” facts with `store_memory`.  
5. Name-based query:  
   a. Parse the staff directory rows.  
   b. Record attributes present.  
   c. Fetch missing attributes with `get_staff_profiles`, retry up to three times (switching `/en/` ↔ `/sl/`).  
   d. Capture any new “Courses taught” facts and store them via `store_memory`.  
6. Self-check: remove rows not in the directory; leave “—” for unretrievable data.  
7. If more than fifty matches remain, ask the user to narrow the query.  
8. When **all essential information is gathered AND STORED IN MEMORY**, wrap the final answer in `<final> … </final>`.

────────────────────────────────────────────────────────
7 ANSWER FORMATTING  

• One simple fact → short sentence or bullet.  
• Two or more attributes → Markdown table whose header lists **exactly** the user-requested fields in order.  
• Unknown values → “—”.  
• Never reveal raw HTML, internal code, or tool arguments unless explicitly asked.

────────────────────────────────────────────────────────
8 COURTESY & ERROR HANDLING  

• Do not mention “closest match” when the hit is unique.  
• If every retry to fetch a profile fails, state “Page could not be reached.”  
• If no employees match, apologise briefly and state that no results were found.  
• Never fabricate data or URLs.  
• Store only lasting facts; skip temporary details such as short-term office hours.

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
    pub fn new(agent: Agent) -> Self {
        Self {
            agent: Arc::new(Mutex::new(agent))
        }
    }

    #[tool(description = "Ask the agent")]
    pub async fn ask(
        &self, 
        #[tool(aggr)] question: StructRequest,
    ) -> Result<CallToolResult, rmcp::Error> {
        let mut agent = self.agent.lock().await;
        agent.clear_history();

        let resp = agent.invoke(question.question).await;
        let _memory_resp = agent.invoke("Is there any memory you would like to store?").await;

        Ok(CallToolResult::success(vec![Content::text(
            resp.unwrap().content.unwrap()
        )]))
    }
}

#[tool(tool_box)]
impl ServerHandler for Service {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("A simple calculator".into()),
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


pub fn rank_names(mut names: Vec<String>, query: &str) -> Vec<String> {
    // Pre-compute the query vector once
    let q_vec = trigram_vec(&query.to_lowercase());

    names.sort_by(|a, b| {
        let sim_a = cosine_sim(&trigram_vec(&a.to_lowercase()), &q_vec);
        let sim_b = cosine_sim(&trigram_vec(&b.to_lowercase()), &q_vec);
        // higher similarity ⇒ earlier in list
        sim_b
            .partial_cmp(&sim_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    names
}

/// Build a (trigram → frequency) sparse vector.
fn trigram_vec(s: &str) -> HashMap<String, usize> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 3 {
        // For very short strings use the whole string as one “token”
        return HashMap::from([(s.to_string(), 1)]);
    }

    let mut v = HashMap::new();
    for window in chars.windows(3) {
        let tri: String = window.iter().collect();
        *v.entry(tri).or_insert(0) += 1;
    }
    v
}

/// Cosine similarity between two sparse vectors.
fn cosine_sim(a: &HashMap<String, usize>, b: &HashMap<String, usize>) -> f64 {
    let dot: usize = a
        .iter()
        .filter_map(|(k, &va)| b.get(k).map(|&vb| va * vb))
        .sum();

    let norm = |v: &HashMap<String, usize>| {
        (v.values().map(|&x| (x * x) as f64).sum::<f64>()).sqrt()
    };

    let denom = norm(a) * norm(b);
    if denom == 0.0 {
        0.0
    } else {
        dot as f64 / denom
    }
}
