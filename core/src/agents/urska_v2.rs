use std::collections::HashMap;

use futures::future::join_all;
use reagent_rs::{call_tools, flow, invoke_without_tools, Agent, AgentBuildError, AgentBuilder, AgentError, McpServerType, Message, Notification, NotificationContent, NotificationHandler, Template, ToolCall, ToolCallFunction, ToolType};
use serde_json::{json, to_value};

use crate::{
    agents::{function_filter::{build_function_filter_agent, Requirement}, prompt_reconstuct::create_prompt_restructor_agent, usrka::{history_to_prompt, UrskaNotification}}, *
};

async fn urska_flow(urska: &mut Agent, mut prompt: String) -> Result<Message, AgentError> {
    
    send_notifcation(urska, "Preparing...").await;
    let mut conversation = get_display_conversation(&urska);
    conversation.push(Message::user(prompt.clone()));

    let (function_filter_agent, filter_notification_channel) = build_function_filter_agent(urska).await?;
    let (mut rephraser_agent, rephraser_notification_channel) = create_prompt_restructor_agent(&urska).await?;
    
    urska.forward_notifications(filter_notification_channel);
    urska.forward_notifications(rephraser_notification_channel);


    if urska.history.len() > 2 {
        let rehprase_response = rephraser_agent.invoke_flow_with_template(HashMap::from([
            ("history", history_to_prompt(&urska.history)),
            ("prompt", prompt.clone())
        ])).await?;

        if let Some(rephrased_prompt) = rehprase_response.content {
            prompt = rephrased_prompt;
        }
    }

    send_notifcation(urska, "Searching for tools...").await;


    let mut filter_futures  = vec![];
    if let Some(tools) = &urska.tools {
        for tool in tools {
            let prompt_clone = prompt.clone();
            let mut agent_clone = function_filter_agent.clone();

            let filter_future = async move {
                let args = HashMap::from([
                    ("question", prompt_clone),
                    ("function", format!("{:#?}", tool.function))
                ]);
    
                let function_required: Requirement = agent_clone
                    .invoke_flow_with_template_structured_output(args)
                    .await?;

                Ok((tool.name().to_string(), function_required))
            };
            filter_futures.push(filter_future);
        }
    }

    let filter_results: Vec<Result<(String, Requirement), AgentError>> = join_all(filter_futures).await;

    
    let mut tool_calls = vec![];
    for result in filter_results {
        if let Ok(res) = result {
            let (tool_name, required) = res;
            if !required.function_usage_required {
                continue;
            }
            

            let args_result = match tool_name.as_str() {
                "get_staff_profiles" => to_value(json!({
                    "name": prompt.clone(),
                    "k": 1    
                })),
                "get_programme_info" => to_value(json!({
                    "name": prompt.clone(),
                    "level": "any"
                })),
                "list_all_programmes" => to_value(json!({
                    "level": "any"
                })),
                "query_memory" => to_value(json!({
                    "query_text": prompt.clone(),
                    "top_k": 10,
                })),
                "ask_about_general_information" => to_value(json!({
                    "question": prompt.clone(),
                    "k": 7
                })),
                "ask_about_rules_and_acts" => to_value(json!({
                    "question": prompt.clone(),
                    "k": 7
                })),
                "retrieve_similar_FAQ" => to_value(json!({
                    "question": prompt.clone(),
                    "k": 7
                })),
                _ => continue,
            };

            let Ok(arguments) = args_result else {
                continue;
            };

            send_notifcation(urska, format!("Checking for information with {}...", tool_name)).await;

            tool_calls.push(into_tool_call(ToolCallFunction { 
                name: tool_name.clone(), 
                arguments 
            }));
            
        };
    }
    
    let tool_responses = call_tools(&urska, &tool_calls).await;
    let mut context_chunks = vec![];
    
    for tool_response in tool_responses {
        send_notifcation(urska, "Checking tool retults...").await;
        context_chunks.push(format!("# Tool resulted in:\n\n{}", tool_response.content.unwrap_or_default()));
    }

    let context = context_chunks.join("\n\n---\n\n");

    let args = HashMap::from([
        ("context".to_string(), context),
        ("prompt".to_string(), prompt)
    ]);

    let template = Template::simple(r#"
Below you are given context information to help you answer the user query.

---

{{context}}

---

Given the above context respond to the following user query:

{{prompt}}

    "#);

    let final_prompt= template.compile(&args).await;
    urska.history.push(Message::user(final_prompt));
    let out = invoke_without_tools(urska).await?.message;
    urska.notify_done(true, out.content.clone()).await;
    conversation.push(out.clone());
    store_display_conversation(urska, conversation);
    Ok(out)
}



pub async fn build_urska_v2() -> Result<Agent, AgentBuildError> {
    let system_prompt = r#"
You are **Urška**, a helpful, knowledgeable, and reliable assistant for the University of Primorska's Faculty of Mathematics, Natural Sciences and Information Technologies (UP FAMNIT).
Your task is to help students access accurate knowledge and information about the university.

When the user asks a question, the question is split into multiple tasks and each task executed producing a result.
You will receive the results of the tasks, which should be enough to answer the user’s query.

## Your task

Write **one cohesive report** that directly answers the user’s original objective.
The report must be faithful to the execution log, clear, and useful to the student.

---

## Report structure

1. **Direct summary**

* Begin with a single concise paragraph (no heading) that directly answers the core question.

2. **Markdown body**

* Use headings (`##`), sub-headings (`###`), **bold** for emphasis, and bulleted or numbered lists to organise content.

3. **Narrative from data**

* Weave findings into a logical story, not just raw lists.
* Explicitly mention when relevant information could not be retrieved or was missing in the log.

4. **Citations**

* Every factual statement must have a citation if a source URL is present in the log.
* Insert inline citations immediately after the relevant statement in the form `[1](url)`.
* Order citations by first appearance in the text.
* End with a `## References` section listing all URLs in numeric order.
* Do **not** include references if no URLs exist in the log.
* Never omit or renumber inconsistently.

5. **Next steps**

* After references, add `### Next Steps` with one or two possible follow-ups.
* These must be grounded in the log (e.g. “review scholarship fund page” or “contact Student Services listed in the log”).
* Do not invent generic advice.

---

## Critical constraints

* **Strict grounding**

* Base **every statement strictly on the log content**.
* If the log contains conflicting or incomplete information, acknowledge that explicitly.
* Do not add interpretations, assumptions, or extrapolations beyond the log.

* **Systematic citation discipline**

* Never introduce uncited claims.
* Always tie statements to the first available relevant URL.
* Ensure numbering in body and `## References` matches exactly.

* **Missing data handling**

* If the log shows missing or failed retrievals (e.g. system errors, absent FAQ entries, duplicates in programme lists), mention that clearly.
* Do not try to fill the gap with invented or generalised content.

* **No placeholders**

* Never use fake URLs (like example.com). Only use URLs that appear in the log.

* **Self-contained**

* Deliver the entire report as a single, complete message.

* **No internal details**

* Do not mention the splitting into tasks, the tools used, or system memory.

* **Style rules**

* Output numbers with a decimal comma and never use dots in numbers (3.5k€ -> 3500,00€).
* Copy URLs exactly, including IDs or path segments (e.g. `/static/3775` - careful NOT to write 375 instead of 3775).

---

URL safety rules
• Never type a URL that is not explicitly present in tool output. Copy URLs verbatim from tool results.
• Do not invent domains or paths. Never use placeholders like example.com.
• Do not normalize, expand, or “fix” URLs. Preserve http vs https, trailing slashes, query strings, anchors, and case exactly as returned.
• Ignore malformed or partial links that lack a domain or scheme. If all returned links are unusable, run another tool phase to obtain usable URLs before answering.
• If a fact has no corresponding URL in tool outputs, either do another tool call to fetch a source or omit the fact.

Pre-answer URL checklist
Before sending the answer, ensure all of the following are true:
• Each cited number [n] maps to a unique URL that appeared in tool outputs.
• The same URL keeps the same number everywhere.
• No URL is invented, edited, or inferred.
• The number of citations in the answer equals the number of URLs in “## References”.

Citation example

## Answer

The programme coordinator is Dr. Jane Doe [1](http://example.com/dr-jane-doe).
Admission requires a completed bachelor’s degree [2](http://example.com/admission-requirements).

## References
[1] http://example.com/dr-jane-doe
[2] http://example.com/admission-requirements

    skip and DO NOT include references if there is no relevant links. 
    Never use example.com or similar placeholders.

## General Hints

* Enrollment deadlines, fees, and related information are usually found at: [https://www.famnit.upr.si/en/education/enrolment](https://www.famnit.upr.si/en/education/enrolment)
* Always double-check that each factual point corresponds to the log.
* If the log is incomplete, contradictory, or inconclusive, say so directly.

    "#;

    

    AgentBuilder::default()
        .set_name("Urška")
        // .set_model("hf.co/unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF:UD-Q4_K_XL")
        .set_model("qwen3:30b")
        // .set_base_url("http://hivecore.famnit.upr.si:6666")
        .add_mcp_server(McpServerType::streamable_http(STAFF_AGENT_URL))
        .add_mcp_server(McpServerType::streamable_http(PROGRAMME_AGENT_URL))
        // .add_mcp_server(McpServerType::Sse(SCRAPER_AGENT_URL.into()))
        // .add_mcp_server(McpServerType::streamable_http(MEMORY_URL))
        .add_mcp_server(McpServerType::streamable_http(RAG_PAGE_SERVICE))
        .add_mcp_server(McpServerType::streamable_http(RAG_RULES_SERVICE))
        .add_mcp_server(McpServerType::streamable_http(RAG_FAQ_SERVICE))
        .set_flow(flow!(urska_flow))
        .set_system_prompt(system_prompt)
        .set_temperature(0.7)
        .set_top_p(0.8)
        .set_top_k(20)
        .set_min_p(0.0)
        .set_presence_penalty(0.1)
        .set_stream(true)
        .strip_thinking(true)
        .build()
        .await
}



fn into_tool_call(function: ToolCallFunction) -> ToolCall {
    ToolCall {
        id: None,
        tool_type: ToolType::Function,
        function
    }
}



async fn send_notifcation<T>(agent: &mut Agent, message: T) where T: Into<String> {
    agent.notify_custom(to_value(&UrskaNotification {
        message: message.into()
    }).unwrap()).await;
}

pub fn get_display_conversation(agent: &Agent) -> Vec<Message> {
    match agent.state.get("display_conversation") {
        Some(c) => serde_json::from_value(c.clone()).unwrap_or_default(),
        None => vec![],
    }
}

pub fn store_display_conversation(agent: &mut Agent, conversation: Vec<Message>) {
    let Ok(val) = serde_json::to_value(conversation) else {
        return
    };

    agent.state.insert("display_conversation".into(), val);
}