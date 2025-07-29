use std::collections::HashMap;
use anyhow::Result;
use reagent::{Message, Role};
use reqwest::Url;
use rmcp::{model::{CallToolRequestParam, ClientCapabilities, ClientInfo, Implementation}, transport::SseClientTransport, ServiceExt};
use scraper::{Html, Selector};

use crate::{programme::{Programme, ProgrammeLevel}, BASE_URL, MEMORY_MCP_URL};


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


pub async fn get_page<T>(url: T) -> Result<String> where T: Into<String> {
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

pub fn parse_programme_list_page(html: &str, level: ProgrammeLevel) -> Vec<Programme> {
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


pub fn history_to_memory_prompt(history: Vec<Message>) -> String {
    let mut prompt = String::from("Here is a summary of a conversation.");
    for msg in history.iter().skip(2) { // Skip the system prompt and the initial memory query result
        let content = msg.content.clone().unwrap_or_default();
        match msg.role {
            Role::User => prompt.push_str(&format!("USER ASKED: {}\n\n", content)),
            Role::Assistant => prompt.push_str(&format!("ASSISTANT: {}\n\n", content)),
            Role::Tool => {
                let tool_name = msg.tool_call_id.as_deref().unwrap_or("unknown_tool");
                prompt.push_str(&format!("TOOL `{:?}` RETURNED:\n{}\n\n", tool_name, content));
            }
            Role::System => continue,
        }
    }
    prompt.push_str("---\nEnd of conversation summary.");

    println!("CONVO: {}", prompt);
    prompt
}



pub async fn get_memories(arguments: serde_json::Value) -> Result<String> {
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