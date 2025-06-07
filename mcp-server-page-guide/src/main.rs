use std::{collections::HashMap, sync::Arc};

use reagent::{init_default_tracing, Agent, AgentBuilder, AsyncToolFn, McpServerType, ToolBuilder, ToolExecutionError, Value};
use rmcp::{
    model::{CallToolRequestParam, CallToolResult, ClientCapabilities, ClientInfo, Content, Implementation, ServerCapabilities, ServerInfo}, schemars, tool, transport::{SseClientTransport, SseServer}, ServerHandler, ServiceExt
};
use anyhow::Result;
use scraper::{Html, Selector};
use serde::Deserialize;
use tokio::sync::Mutex;


const BIND_ADDRESS: &str = "127.0.0.1:8001";


#[tokio::main]
async fn main() -> Result<()> {

   
    init_default_tracing();
    let agent_system_prompt = r#"
    You are **UniStaff-Agent**, an assistant that answers questions about university employees.

    ---

    ## INPUT CONTEXT  
    * At the start of each conversation you receive the scraped HTML of the main staff-directory page.  
    * Every row holds **surname**, **given name**, and when available **e-mail**, **phone** and an **href** pointing to the full profile.  
    * You can call **get_web_page_content(url: string) → html** to fetch any extra public page on the same host (*famnit.upr.si*).  
    * Most (not all) personal links follow  
    `https://www.famnit.upr.si/en/about-faculty/staff/<firstname>.<lastname>`.

    ---

    ## WORKFLOW  
    1. **Language detection** – identify whether the request is in Slovenian or English and reply in that language.  

    2. **Ambiguity check** – if the request mentions one or *several* persons and any of them is underspecified:
    * Split the query into individual name fragments (comma/“and”/newline separators).
    * For **each** fragment:
        * search the staff list with fuzzy matching (initials, diacritics, minor typos);
        * if exactly **one** hit is found, accept it silently and continue – do **not** warn that the name was “not found”;
        * if several hits remain, collect them and ask **one concise clarifying question** before tool calls;
        * if zero hits remain after fuzzy search, apologise briefly for that specific name only.
    * Proceed to answer for **all successfully resolved names** even if some other names still need clarification.

    3. **Query analysis** – extract the requested attributes (phone, e-mail …) and all filters (name fragment, department, e-mail domain …).  

    4. **Local parsing** – examine the supplied staff-list HTML and select rows that satisfy the filters.  

    5. **Attribute completion** for each selected employee  
    1. Record attributes already present in the row.  
    2. If any requested attribute is missing, fetch the profile with **get_web_page_content**. You may retry this multiple times. 
    3. On failure, retry up to **three** times with back-off and simple heuristics (e.g. switch language prefix `/sl/` ↔ `/en/`).  

    6. **Self-check & correction**  
    * Verify that every row in the draft answer matches a name present in the original list; **remove any entry that is not**.  
    * Re-parse cached HTML with alternative selectors if something is still missing.  
    * If an attribute cannot be recovered, leave the placeholder “—”.  

    7. **Result limiting** – if more than 50 matches remain, stop, inform the user and ask for narrower criteria.  

    8. **Compose the answer** following the output rules below.

    ---

    ## OUTPUT FORMAT  
    * Respond in **Markdown**.  
    * If two (2) or more attributes are returned, output a table whose header lists **exactly** the user-requested fields in the same order.  
    * Missing values → “—”.  
    * If the user asked for a single simple fact, a short sentence or bullet is enough.  
    * Never invent data or reveal raw HTML or tool arguments unless explicitly asked.  
    * When you are ready to respond to the user, wrap the final answer inside `<final>…</final>` tags.

    ---

    ## COURTESY & ERROR HANDLING  
    * Never report “closest match is X” when X already *is* a unique fuzzy hit – just use X.  
    * If the query mentions multiple names, return results for the ones you could resolve and clearly list the names that could **not** be matched, asking the user only about those.
    * If no rows match, apologise briefly and state that no results were found.  
    * If a fetch ultimately fails, report that the page could not be reached.  
    * Avoid repeat tool calls; never guess or fabricate URLs or names.

    ---

    "#;
    
    // let stop_prompt = r#"
    // **Decision Point:**

    // Your previous output is noted. Now, explicitly decide your next step:

    // 1.  **Continue Working:** If you need to perform more actions (like calling a tool such 
    // as `add_memory`, `get_current_weather`, `query_memory`, or doing more internal 
    // reasoning), clearly state your next specific action or internal thought. **Do NOT use 
    // `<final>` tags for this.**

    // 2.  **Final Answer:** If you completed all the tasks and want to submit the final answer 
    // for the user, re-send that **entire message** now, but wrap it in `<final>Your final message 
    // here</final>` tags.

    // Choose one option.
    // "#;

    let staff_list_result = get_page("https://www.famnit.upr.si/en/about-faculty/staff/").await;
    let all_names = match staff_list_result {
        Ok(staff_list) => staff_html_to_markdown(&staff_list).1,
        Err(e) => return Err(anyhow::anyhow!("Fetching employee list error: {:#?}", e.to_string()))
    };

    let similar_names_executor: AsyncToolFn = {
        Arc::new(move |args: Value| {
            let names = all_names.clone();
            Box::pin(async move {
                let names = names.clone();
                
                let query_name = args.get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolExecutionError::ArgumentParsingError("Missing 'name' argument".into()))?;

                let k = args.get("k")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| 5);

                let names = rank_names(names, query_name)[0..k as usize].to_vec();
                Ok(names.join("\n"))
            })
        })
    };

    let similar_names_tool = ToolBuilder::new()
        .function_name("get_top_k_similar_names")
        .function_description("Given a name and optionally k (default 5), the tool returns top k similar \
        names of employees to the queried name, based on levenstein distance.")
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
        .set_stopword("<final>")
        .add_tool(similar_names_tool)
        // .set_stop_prompt(stop_prompt)
        .build()
        .await?;



        
    
        
    

    let ct = SseServer::serve(BIND_ADDRESS.parse()?)
        .await?
        .with_service(move || Service::new(agent.clone()));

    tokio::signal::ctrl_c().await?;
    ct.cancel();

    Ok(())

   
    // println!("{:#?}", tool_result);

    // Ok(())
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
        println!("{:#?}", agent);
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

    let tools = client.list_tools(Default::default()).await?;
    println!("Available tools: {tools:#?}");


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

pub fn staff_html_to_markdown(html: &str) -> (String, Vec<String>) {
    let doc     = Html::parse_document(html);
    let row_sel = Selector::parse("#osebje-list tr").unwrap();
    let td_sel  = Selector::parse("td").unwrap();
    let a_sel   = Selector::parse("a").unwrap();

    let mut out = Vec::new();
    let mut names = vec![];
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
        names.push(format!("{} {}", given, surname));
        out.push(line);
    }

    (out.join("\n"), names)
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
