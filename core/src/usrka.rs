use std::collections::HashMap;

use futures::future::join_all;
use reagent_rs::{flow, invoke_without_tools, Agent, AgentBuildError, AgentBuilder, AgentError, Flow, FlowFuture, McpServerType, Message, Notification, NotificationContent, Role, StatelessPrebuild, Template};
use rmcp::transport::worker;
use serde::Serialize;
use serde_json::{json, to_value};

use crate::{
    blueprint::create_blueprint_agent, 
    executor::create_single_task_agent, 
    planner::{create_planner_agent, Plan}, 
    prompt_reconstuct::create_prompt_restructor_agent, 
    quick_responder::{create_quick_response_agent, Answerable}, 
    replanner::create_replanner_agent, 
    MEMORY_URL, PROGRAMME_AGENT_URL, RAG_FAQ_SERVICE, 
    RAG_PAGE_SERVICE, RAG_RULES_SERVICE, SCRAPER_AGENT_URL, 
    STAFF_AGENT_URL
};

#[derive(Debug, Clone, Serialize)]
pub struct UrskaNotification {
    message: String,
}

pub async fn plan_and_execute_flow(agent: &mut Agent, mut prompt: String) -> Result<Message, AgentError> {
    agent.notify(NotificationContent::Custom(to_value(&UrskaNotification {
        message: "Preparing...".into()
    }).unwrap())).await;

    agent.history.push(Message::user(prompt.clone()));
    let mut inner_iterations_bound = 100;

    let mut past_steps: Vec<(String, String)> = Vec::new();
    let mut flow_histroy: Vec<Message> = vec![Message::system(agent.system_prompt.clone())];
    
    let (mut quick_responder_agent, quick_responder_notification_channel) = create_quick_response_agent(&agent).await?;
    let (mut rephraser_agent, rephraser_notification_channel) = create_prompt_restructor_agent(&agent).await?;
    let (mut blueprint_agent, blueprint_notification_channel) = create_blueprint_agent(agent).await?;
    let (mut planner_agent, planner_notification_channel) = create_planner_agent(agent).await?;
    let (mut replanner_agent, replanner_notification_channel) = create_replanner_agent(agent).await?;
    let (mut executor_agent, executor_notification_channel) = create_single_task_agent(agent).await?;

    agent.forward_notifications(quick_responder_notification_channel);
    agent.forward_notifications(rephraser_notification_channel);
    agent.forward_notifications(blueprint_notification_channel);
    agent.forward_notifications(planner_notification_channel);
    agent.forward_notifications(replanner_notification_channel);
    agent.forward_notifications(executor_notification_channel);      

    // more than system + first prompt
    // query rewrite
    if agent.history.len() > 2 {
        agent.notify(NotificationContent::Custom(to_value(&UrskaNotification {
            message: "Assessing query...".into()
        }).unwrap())).await;



        let rehprase_response = rephraser_agent.invoke_flow_with_template(HashMap::from([
            ("history", history_to_prompt(&agent.history)),
            ("prompt", prompt.clone())
        ])).await?;

        if let Some(rephrased_prompt) = rehprase_response.content {
            prompt = rephrased_prompt;
        }
    }

    // if we have access to FAQ, check if we can answer right away using FAQ
    let mut FAQ = None;
    if let Some(tool) = agent.get_tool_ref_by_name("retrieve_similar_FAQ") {
        quick_responder_agent.notify(NotificationContent::Custom(to_value(&UrskaNotification {
            message: "Checking for quick response...".into()
        }).unwrap())).await;


        let faq = match tool.execute(json!({
            "question": prompt,
            "k": 10,
        })).await {
            Ok(resp) => resp,
            Err(_e) => "No similar FAQ found".into(),
        };

        quick_responder_agent
            .notify(NotificationContent::ToolCallSuccessResult(faq.clone()))
            .await;

        let input = HashMap::from([
            ("prompt", prompt.clone()),
            ("faq",  faq.clone().into())
        ]);

        let answ: Answerable = quick_responder_agent
            .invoke_flow_with_template_structured_output(input)
            .await?;

        flow_histroy.push(Message::tool(faq.clone(), "1"));

        if answ.can_respond {
            FAQ = Some(faq);
        }
    };

    // create a general plan on how to tackle the problem
    let blueprint = blueprint_agent.invoke_flow_with_template(HashMap::from([
        ("tools", format!("{:#?}", agent.tools)),
        ("prompt", prompt.clone()),
        ("faq", format!("{:#?}", FAQ)),
    ])).await?;


    let Some(blueprint) = blueprint.content else {
        return Err(AgentError::Runtime("Blueprint was not created".into()));
    };


    if FAQ.is_none() {
        planner_agent.notify(NotificationContent::Custom(to_value(&UrskaNotification {
            message: "Thinking how to find the requested information...".into()
        }).unwrap())).await;


        // create a detailed step by step plan on how to tackle the problem
        let plan: Plan = planner_agent.invoke_flow_with_template_structured_output(HashMap::from([
            ("tools", format!("{:#?}", agent.tools)),
            ("prompt", blueprint)
        ])).await?;


        // save plan to file
        serde_json::to_writer_pretty(std::fs::File::create("last_plan.json").unwrap(), &plan).unwrap();


        let mut i = 0;
        let mut executor_fututres = vec![];
        for step_sequence in plan.steps.into_iter() {
            let worker_clone = executor_agent.clone();
            // let prompt_clone = prompt.clone();
            let executor_future = async move {
                let mut worker = worker_clone;
                let mut executor_task_log = vec![];

                for step in step_sequence.into_iter() {
                    worker.notify(NotificationContent::Custom(to_value(&UrskaNotification {
                        message: step.clone()
                    }).unwrap())).await;

                    let response = match worker.invoke_flow(step.clone()).await {
                        Ok(resp) => resp,
                        Err(e) => {
                            println!("Error executing step `{}`: {}", step, e);
                            continue;
                        },
                    };
                    executor_task_log.push((
                        Message::user(step.clone()), 
                        response
                    ));
                }

                let _ = worker.save_history(format!("executor_run_{}_conversation.json", i));
                executor_task_log

            };
            i += 1;
            executor_fututres.push(executor_future);
        
        }

        agent.notify(NotificationContent::Custom(to_value(&UrskaNotification {
            message: "Constructing answer...".into()
        }).unwrap())).await;

        let executor_results = join_all(executor_fututres).await;
        let mut past_steps: Vec<(String, String)> = Vec::new();

        for executor_task_log in executor_results {
            for (task, response) in executor_task_log {
                past_steps.push((
                    task.content.unwrap_or_default(), 
                    response.content.unwrap_or_default()
                ));
            }
    
        }
        
        let aggregated_history = past_steps
            .iter()
            .enumerate()
            .map(|(i, (task, response))| {
                format!(
                    "### Step {}\nUser Instruction:\n{}\n\nExecutor Response:\n{}\n ",
                    i + 1,
                    task.trim(),
                    response.trim()
                )
            })
            .collect::<Vec<String>>()
            .join("\n\n---\n\n");

        flow_histroy.push(Message::tool(aggregated_history, "0"));



    }

    


    
    flow_histroy.push(Message::user(prompt));


    let mut conversation_history = agent.history.clone();
    agent.history = flow_histroy;
    
    let response = invoke_without_tools(agent).await?;
    conversation_history.push(response.message.clone());
    
    let _ = agent.save_history("urska_conversation.json".to_string());

    agent.history = conversation_history;

    agent.notify(NotificationContent::Done(true, response.message.content.clone())).await;
    Ok(response.message)
}


pub async fn build_urska() -> Result<Agent, AgentBuildError> {

    let system_prompt = r#"
You are **Urška**, a helpful, knowledgeable, and reliable assistant for the University of Primorska's Faculty of Mathematics, Natural Sciences and Information Technologies (UP FAMNIT).
Your task is to help students access accurate knowledge and information about the university.

When the user asks a question, the question is split into multiple tasks and each task executed producing a result.
You will receive the results of the tasks, which should be enough to answer the user’s query.

---

## What you will receive

* A conversation history in which

1. `User` messages describe tasks that were executed.
2. `Assistant` messages contain the raw results, observations, and any source URLs.

The **final `User` message** in the log restates the objective and tells you to begin.

---

## Your final task

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

  * If numbers in the source use commas for decimals, output with a dot.
  * Copy URLs exactly, including IDs or path segments (e.g. `/static/3775`).

---

## General Hints

* Enrollment deadlines, fees, and related information are usually found at: [https://www.famnit.upr.si/en/education/enrolment](https://www.famnit.upr.si/en/education/enrolment)
* Always double-check that each factual point corresponds to the log.
* If the log is incomplete, contradictory, or inconclusive, say so directly.

    "#;

    AgentBuilder::default()
        .set_system_prompt(system_prompt)
        .set_flow(flow!(plan_and_execute_flow))
        .set_model("hf.co/unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF:UD-Q4_K_XL")

        // .set_model("gemma3:270m")
        // .set_model("gpt-oss:120b")
        // .set_model("qwen3:0.6b")
        .set_name("Urška")
        .set_base_url("http://hivecore.famnit.upr.si:6666")
        .add_mcp_server(McpServerType::streamable_http(STAFF_AGENT_URL))
        .add_mcp_server(McpServerType::streamable_http(PROGRAMME_AGENT_URL))
        .add_mcp_server(McpServerType::Sse(SCRAPER_AGENT_URL.into()))
        .add_mcp_server(McpServerType::streamable_http(MEMORY_URL))
        .add_mcp_server(McpServerType::streamable_http(RAG_PAGE_SERVICE))
        .add_mcp_server(McpServerType::streamable_http(RAG_RULES_SERVICE))
        .add_mcp_server(McpServerType::streamable_http(RAG_FAQ_SERVICE))
        .set_temperature(0.7)
        .set_top_p(0.8)
        .set_top_k(20)
        .set_min_p(0.0)
        .set_presence_penalty(0.1)
        .set_max_iterations(2)
        .set_stream(true)
        .build()
        .await
        
}



pub fn history_to_prompt(history: &Vec<Message>) -> String {
    let mut prompt = String::from("Here is a summary of a conversation.");
    for msg in history.iter().skip(1) {
        let content = msg.content.clone().unwrap_or_default();
        match msg.role {
            Role::User => prompt.push_str(&format!("USER ASKED: {}\n\n", content)),
            Role::Assistant => prompt.push_str(&format!("ASSISTANT: {}\n\n", content)),
            Role::Tool => {
                        let tool_name = msg.tool_call_id.as_deref().unwrap_or("unknown_tool");
                        prompt.push_str(&format!("TOOL `{:?}` RETURNED:\n{}\n\n", tool_name, content));
                    }
            Role::System => continue,
            Role::Developer => continue,
        }
    }
    prompt.push_str("---\nEnd of conversation summary.");

    println!("CONVO: {}", prompt);
    prompt
}

