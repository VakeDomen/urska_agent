use std::collections::HashMap;

use futures::future::join_all;
use reagent::{
    error::{AgentBuildError, AgentError}, flow_types::{Flow, FlowFuture}, invocations::invoke_without_tools, json, util::Template, Agent, AgentBuilder, McpServerType, Message, NotificationContent, Role
};
use rmcp::transport::worker;

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

pub(crate )fn plan_and_execute_flow<'a>(agent: &'a mut Agent, mut prompt: String) -> FlowFuture<'a> {
    Box::pin(async move {
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


            if answ.can_respond {
                FAQ = Some(faq.clone());
                flow_histroy.push(Message::tool(faq, "1"));
            }
        };

        // // create a general plan on how to tackle the problem
        // let blueprint = blueprint_agent.invoke_flow_with_template(HashMap::from([
        //     ("tools", format!("{:#?}", agent.tools)),
        //     ("prompt", prompt.clone()),
        //     ("faq", format!("{:#?}", FAQ)),
        // ])).await?;


        // let Some(blueprint) = blueprint.content else {
        //     return Err(AgentError::Runtime("Blueprint was not created".into()));
        // };


        if FAQ.is_none() {
            // create a detailed step by step plan on how to tackle the problem
            let plan: Plan = planner_agent.invoke_flow_with_template_structured_output(HashMap::from([
                ("tools", format!("{:#?}", agent.tools)),
                ("prompt", prompt.clone())
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

                    let _ = worker.save_history(format!("executor_run_{}.json", i));
                    executor_task_log
    
                };
                i += 1;
                executor_fututres.push(executor_future);
            
            }
    
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
    


        } else {
            flow_histroy.push(Message::tool(FAQ.unwrap_or_default(), "1"));
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
    })    
}


pub async fn build_urska() -> Result<Agent, AgentBuildError> {

    let system_prompt = r#"
    You are **Urška**, a helpful, knowledgeable, and reliable assistant for the University of Primorska's Faculty of Mathematics, Natural Sciences and Information Technologies (UP FAMNIT).
    Your task is to help students access the knowledge and information about the university. 

    When the user asks a question, the question is split into multiple tasks and each task executed producing a result.
    You will recieve the results of the task, which hopefully are enough to answe the user's query.

    ### What you will receive
    * A conversation history in which  
    1. `User` messages describe tasks that were executed.  
    2. `Assistant` messages contain the raw results, observations, and any source URLs.

    ### Your final task
    Write **one cohesive report** that directly answers the user’s original objective.  
    The final `User` message in the log restates that objective and tells you to begin.

    ---

    ## Report structure

    1. **Direct summary**  
    Open with a single concise paragraph (no heading) that answers the core question.

    2. **Markdown body**  
    Use headings (`##`), sub‑headings (`###`), **bold** for emphasis, and bulleted or numbered lists to organise the rest of the content.

    3. **Narrative from data**  
    Weave the key findings into a logical story. Do **not** simply list results.

    4. **Citations**  
    * Extract source URLs from the execution log.  
    * Attach an inline citation immediately after each sourced fact, using a numbered link: `[1](http://example.com)`.  
    * End the report with a `## References` section listing the full URLs in numeric order.  

    *Citation example*  

    > The programme coordinator is Dr. Jane Doe [1](http://example.com/dr‑jane‑doe).  
    > Admission requires a completed bachelor’s degree [2](http://example.com/admission‑requirements).  
    >  
    > ## References  
    > [1] http://example.com/dr‑jane‑doe  
    > [2] http://example.com/admission‑requirements  

    5. **Next steps**  
    After the references, add `### Next Steps` with one or two helpful follow‑up questions or actions.

    ---

    ## Critical constraints

    * **Never mention your internal process or the tools used**; focus solely on providing the user with the 
    information that was uncovered and the user might want to know.  
    * **Base every statement strictly on the log content**.   
    * Deliver the entire report as a single, self‑contained message.
    * Include all relevant links. All links included should be existing links and found in the conversations. 

    skip and DO NOT include references if there is no relevant links. 
    Never use example.com or similar placeholders.

    GENERAL HINTS:
    - https://www.famnit.upr.si/en/education/enrolment <- contains enrollement deadlines, links to fees,...
    "#;

    AgentBuilder::default()
        .set_system_prompt(system_prompt)
        .set_flow(Flow::Custom(plan_and_execute_flow))
        .set_model("hf.co/unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF:UD-Q4_K_XL")

        // .set_model("gemma3:270m")
        // .set_model("gpt-oss:120b")
        // .set_model("qwen3:0.6b")
        .set_name("Urška")
        .set_ollama_endpoint("http://hivecore.famnit.upr.si:6666")
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

