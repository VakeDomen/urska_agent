use std::{collections::HashMap, fmt::format};

use serde_json::Value;
use tokio::sync::mpsc::Receiver;

use reagent::{
    models::{agents::flow::invocation_flows::{Flow, FlowFuture}, configs::PromptConfig, AgentBuildError, AgentError}, prebuilds::{statefull::StatefullPrebuild, stateless::StatelessPrebuild}, util::{invocations::{call_tools, invoke_without_tools}, templating::Template}, Agent, AgentBuilder, McpServerType, Message, Notification, NotificationContent
};

use crate::{MEMORY_URL, PROGRAMME_AGENT_URL, RAG_SERVICE, SCRAPER_AGENT_URL, STAFF_AGENT_URL};


pub(crate )fn plan_and_execute_flow<'a>(agent: &'a mut Agent, prompt: String) -> FlowFuture<'a> {
    Box::pin(async move {
        let mut past_steps: Vec<(String, String)> = Vec::new();
        
        let (mut blueprint_agent, blueprint_notification_channel) = create_blueprint_agent(agent).await?;
        let (mut planner_agent, planner_notification_channel) = create_planner_agent(agent).await?;
        let (mut replanner_agent, replanner_notification_channel) = create_replanner_agent(agent).await?;
        let (mut executor_agent, executor_notification_channel) = create_single_task_agent(agent).await?;

        agent.forward_notifications(blueprint_notification_channel);
        agent.forward_notifications(planner_notification_channel);
        agent.forward_notifications(replanner_notification_channel);
        agent.forward_notifications(executor_notification_channel);

        let blueprint = blueprint_agent.invoke_flow_with_template(HashMap::from([
            ("tools", format!("{:#?}", agent.tools)),
            ("prompt", prompt.clone())
        ])).await?;


        let Some(blueprint) = blueprint.content else {
            return Err(AgentError::RuntimeError("Blueprint was not created".into()));
        };


        let plan_content = planner_agent.invoke_flow_with_template(HashMap::from([
            ("tools", format!("{:#?}", agent.tools)),
            ("prompt", blueprint)
        ])).await?;

        let mut plan = get_plan_from_response(&plan_content)?;
        

        
        for iteration in 1.. {
             

            if plan.is_empty() {
                break;
            }

            // put the step instruction to the overarching agent history
            let current_steps = plan.remove(0);
            
            if current_steps.is_empty() {
                break;
            }

            let current_step = if current_steps.len() > 1 {
                format!("# Tasks to complete:\n\n{:#?}", current_steps.join("\n"))
            } else {
                current_steps[0].clone()
            };

            agent.history.push(Message::user(current_step.clone()));

            // execute the step and put response to the overarching agent history
            let response = executor_agent.invoke_flow(current_step.clone()).await?;

            // let mut step_result = format!("# response \n {:#?}", response.content);

            // if let Some(tc) = response.tool_calls.clone() {
            //     for tool_msg in call_tools(&executor_agent, &tc).await {
            //         step_result = format!("{}\n\n## Tool result:\n\n{:#?}", step_result, tool_msg.content);
            //         agent.history.push(tool_msg);
            //     }
            // } 

            

            agent.history.push(response.clone());


            let observation = response.content.clone().unwrap_or_default();
            past_steps.push((current_step, observation));
            // past_steps.push((current_step, step_result));

            
            let past_steps_str = past_steps
               .iter()
               .map(|(step, result)| format!("Step: {step}\nResult: {result}"))
               .collect::<Vec<_>>()
               .join("\n\n");


            if let Some(max_iterations) = agent.max_iterations {
                if iteration >= max_iterations {
                    break;
                }
            }


            let new_plan_content = replanner_agent.invoke_flow_with_template(HashMap::from([
                ("tools", format!("{:#?}", agent.tools)),
                ("prompt", prompt.clone()),
                ("plan", format!("{plan:#?}")),
                ("past_steps", past_steps_str),
            ])).await?;
            plan = get_plan_from_response(&new_plan_content)?;
        }


        if past_steps.last().is_some() {

            agent.history.push(Message::user(prompt.to_string()));
            let response = invoke_without_tools(agent).await?;

            agent.notify(NotificationContent::Done(true, response.message.content.clone())).await;
            Ok(response.message)
        } else {
            agent.notify(NotificationContent::Done(false, Some("Plan-and-Execute failed to produce a result.".into()))).await;
            Err(AgentError::RuntimeError(
                "Plan-and-Execute failed to produce a result.".into(),
            ))
        }
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

    "#;

    AgentBuilder::default()
        .set_system_prompt(system_prompt)
        .set_flow(Flow::Custom(plan_and_execute_flow))
        .set_model("hf.co/unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF:UD-Q4_K_XL")
        // .set_model("qwen3:0.6b")
        .set_name("Urška")
        // .set_ollama_endpoint("http://hivecore.famnit.upr.si:6666")
        .add_mcp_server(McpServerType::streamable_http(STAFF_AGENT_URL))
        .add_mcp_server(McpServerType::streamable_http(PROGRAMME_AGENT_URL))
        .add_mcp_server(McpServerType::Sse(SCRAPER_AGENT_URL.into()))
        .add_mcp_server(McpServerType::streamable_http(MEMORY_URL))
        .add_mcp_server(McpServerType::streamable_http(RAG_SERVICE))
        .set_temperature(0.7)
        .set_top_p(0.8)
        .set_top_k(20)
        .set_min_p(0.0)
        .set_presence_penalty(0.1)
        .set_max_iterations(3)
        .set_stream(true)
        .build()
        .await
        
}

fn get_plan_from_response(plan_response: &Message) -> Result<Vec<Vec<String>>, AgentError> {
    let original_plan_string = plan_response.content.clone().unwrap_or_default();

    let plan_value: Value = serde_json::from_str(&original_plan_string).map_err(|e| {
        AgentError::RuntimeError(format!("Planner failed to return valid JSON: {e}"))
    })?;

    let steps_value = plan_value.get("steps").ok_or_else(|| {
        AgentError::RuntimeError("JSON object is missing the required 'steps' key.".to_string())
    })?;

    let steps: Vec<Vec<String>> = serde_json::from_value(steps_value.clone()).map_err(|e| {
        AgentError::RuntimeError(format!(
            "The 'steps' key is not a valid array of string arrays: {e}"
        ))
    })?;

    Ok(steps)
}

pub async fn create_planner_agent(ref_agent: &Agent) -> Result<(Agent, Receiver<Notification>), AgentBuildError> {
    let ollama_config = ref_agent.export_ollama_config();
    let model_config = ref_agent.export_model_config();
    let prompt_config = if let Ok(c) = ref_agent.export_prompt_config().await {
        c
    } else {
        PromptConfig::default()
    };
    
    let system_prompt = r#"You are a meticulous Tactical Planner Agent. You will be given a high-level **strategy** and the original user **objective/question**. Your **sole purpose** is to convert that strategy into a concise, step-by-step plan in strict JSON format.

# Your Task
Create a JSON object with a single key, `"steps"`.  
• The value of `"steps"` must be an **array of arrays**.  
• Each inner array contains one or more step strings that can be executed **in parallel**.  
• The outer array represents the **sequential order** in which these parallel blocks run.

# Structural Limits
1. Produce **no more than two sequential blocks** of executable work.  
2. Append **one final block** that contains **exactly one summary step**.  
3. Total outer blocks ≤ 3.

# Core Principle  - The Executor Is Blind
The Executor agent receives one inner array at a time and knows nothing about the strategy or objective.  
Therefore every step must be **fully self-contained, explicit, and independent** of hidden context.

# Rules for Plan Creation
1. **Translate Strategy to Tactics** Derive concrete sub-tasks from each phase of the strategy.  
2. **Create Self-Contained Steps** Write clear, imperative instructions. Include all keywords and context from the user’s objective.  
3. **Specify Expected Output** State what information the Executor must return for each step.  
4. **Unknown Information** If a step needs data produced earlier, insert a tag `<<substitute: …>>` where the value will be filled later.  
5. **Final Answer** The last step must read:  
 `"Synthesize all gathered information and provide the final comprehensive answer to the user's objective."`

# Crucial Constraint  - No Generic Steps
Bad `"Use rag_lookup to find information."`  
Good `"Use get_web_page_content to retrieve https://www.famnit.upr.si/en/education/scholarships/UP-scholarship and extract PhD scholarship details, returning source links (use k=2)."`

# Few-Shot Example
**User Objective** Does FAMNIT offer any scholarships to PhD students?  
**High-Level Strategy** Begin with a broad search for doctoral financial aid, list all doctoral programmes, then inspect each programme for scholarship info, and finally synthesize the findings.

**Correct JSON Plan Output**
{
    "steps": [
        [
            "Use rag_lookup and ask 3 distinct questions related to 'FAMNIT scholarships to PhD students' and k=1 for each. Extract any scholarship policies or funding documents and return the text plus relevant links.",
            "Call list_all_programmes with level='doctoral'. Return the list of doctoral programme names offered by the faculty."
        ],
        [
            "For each programme name returned previously, call get_programme_info with name=<<substitute: programme name>>, level='doctoral', sections=['admission_requirements','general_info','financial_support']. Record any explicit mention of scholarships or funding opportunities along with programme name."
        ],
        [
            "Synthesize all gathered information and provide the final comprehensive answer to the user's objective."
        ]
    ]
}

    "#;

    let template = Template::simple(r#"
    # These tools will be avalible to the executor agent: 

    {{tools}}

    Users task to create a JSON plan for: 

    {{prompt}}
    "#);

    StatelessPrebuild::reply_without_tools()
        .import_ollama_config(ollama_config)
        .import_model_config(model_config)
        .import_prompt_config(prompt_config)
        .set_name("Planner")
        .set_response_format(r#"
        {
            "type": "object",
            "properties": {
                "steps": {
                    "type": "array",
                    "items": {
                        "type": "array",
                    "items": {
                        "type": "string"
                    }
                    }
                }
            },
            "required": ["steps"]
        }
        "#)
        .set_system_prompt(system_prompt)
        .set_template(template)
        .set_clear_history_on_invocation(true)
        .set_model(ref_agent.model.clone())
        .build_with_notification()
        .await
}



pub async fn create_blueprint_agent(ref_agent: &Agent) -> Result<(Agent, Receiver<Notification>), AgentBuildError> {
    let ollama_config = ref_agent.export_ollama_config();
    let model_config = ref_agent.export_model_config();
    let prompt_config = if let Ok(c) = ref_agent.export_prompt_config().await {
        c
    } else {
        PromptConfig::default()
    };
    
    let system_prompt = r#"You are a Chief Strategist AI. Your role is to analyze a user's objective 
    and devise a high-level, abstract strategy to achieve it. You do not create step-by-step plans or write code. 
    Your output is a concise, natural language paragraph describing the strategic approach.
    Your strategy will be given to a tactician to plan in detailed steps later.

    # Your Thought Process:
    
    1.  **Understand the Core Goal:** What is the fundamental question the user wants answered?
    
    2.  **Identify Key Information Areas:** What are the major pieces of information needed to reach the 
    goal? (e.g., a date, a name, a location, a technical specification).
    
    3.  **Outline Logical Phases:** Describe the logical flow of the investigation in broad strokes. 
    What needs to be found first to enable the next phase?
    
    4.  **Suggest General Capabilities:** Mention the *types* of actions needed (e.g., "search for 
    historical data," "analyze technical documents," "cross-reference information") without specifying exact 
    tool calls.

    ## Output Rules:
    -   Your entire response MUST be a single, natural language paragraph.
    -   **DO NOT** use JSON.
    -   **DO NOT** create a list of numbered or bulleted steps.
    -   **DO NOT** mention specific tool names like `search_tool` or `rag_lookup`.
    -   **DO NOT** call any tools yourself.

    ---
    **Example 1**

    **User Objective:** "Does famnit offer any scholarships for PhD students?"

    **Correct Strategy Output:**
    To determine if the institution offers scholarships for PhD students, the strategy will begin with a broad search for 
    general information regarding financial aid, funding opportunities, and scholarships specifically related to doctoral 
    studies. This initial phase aims to uncover any overarching policies or documents that are not tied to a single, specific 
    program. Following this general inquiry, the focus will narrow by first identifying all available doctoral-level programmes 
    and then systematically investigating the detailed information for each of those programmes. This more specific lookup will 
    prioritize sections concerning admissions, tuition, and financial support to find explicit mentions of scholarship 
    availability. Finally, the information gathered from both the initial broad search and the subsequent program-specific 
    inquiries will be correlated and synthesized to construct a comprehensive answer.

    "#;

    let template = Template::simple(r#"
    # These tools will later be avalible to the executor agent: 

    {{tools}}

    Users task to create a blueprint for: 

    {{prompt}}
    "#);

    StatelessPrebuild::reply_without_tools()
        .import_ollama_config(ollama_config)
        .import_model_config(model_config)
        .import_prompt_config(prompt_config)
        .set_name("Thinker")
        .set_system_prompt(system_prompt)
        .set_template(template)
        .set_clear_history_on_invocation(true)
        .build_with_notification()
        .await
}



pub async fn create_replanner_agent(ref_agent: &Agent) -> Result<(Agent, Receiver<Notification>), AgentBuildError> {
    let ollama_config = ref_agent.export_ollama_config();
    let model_config = ref_agent.export_model_config();
    let prompt_config = if let Ok(c) = ref_agent.export_prompt_config().await {
        c
    } else {
        PromptConfig::default()
    };
    
    let system_prompt = r#"
You are an expert Re-Planner Agent. Your job is to refine an existing plan so the Executor (who has no knowledge of the objective or history) can finish the user’s task efficiently.

# Core constraints
• The Executor is blind: every new step must be fully self-contained and provide all context needed at the moment it runs.  
• Never repeat steps.  
• Keep the plan as short as possible—add a step only when it is clearly required to reach the objective.  
• The plan must use **array-of-arrays** structure:  
 – Each inner array holds one or more steps that can run **in parallel**.  
 – The outer array lists these parallel blocks in the **order** they must execute.  
• Output at most **two sequential blocks of actionable work** followed by **one final block** that contains exactly **one summarization step** such as “Synthesize all gathered information and provide the final comprehensive answer to the user's objective.”  
• If the plan you receive already consists solely of that summarization block, output `{"steps": []}` unless you are very certain another concrete action is still required.

## Thought process
1. Re-read the original user objective to confirm the end goal.  
2. Examine each past step’s result to determine what is now known, what failed, and what remains.  
3. Substitute any discovered data directly into future steps, replacing placeholders like `<<substitute: …>>` with concrete values.  
4. Remove or rewrite any step that is unnecessary, redundant, or impossible.  
5. If a past step failed, insert at most one corrective step to overcome that specific obstacle.  
6. Stop when only the summarization block is left or when the objective is satisfied.

## Output format
Return a JSON object with a single key, `"steps"`, whose value is an **array of arrays** representing the revised plan.

# Examples

## Example A: Substituting known data
Objective Who was the monarch of the United Kingdom on the date of the first moon landing and what was their full name?  

Original plan  
{
  "steps": [
    ["Find the exact date of the first moon landing and return it."],
    ["Using that date, find the monarch of the United Kingdom at that time and return their common name."],
    ["Synthesize the gathered information and provide the final answer to the user's objective."]
  ]
}

Past results  
[
  { "step": "Find the exact date of the first moon landing and return it.", "result": "July 20 1969" }
]

New plan produced by you  
{
  "steps": [
    ["Find the monarch of the United Kingdom on July 20 1969 and return their common name."],
    ["Synthesize all gathered information and provide the final comprehensive answer to the user's objective."]
  ]
}

## Example B: Pivot after tool failure
Objective Find the email address for the head of the Computer Science Department at FAMNIT.  

Original plan  
{
  "steps": [
    ["Use ask_staff_expert to find the name of the department head."],
    ["Using that name, use ask_staff_expert to find the email address."],
    ["Synthesize the gathered information and provide the final answer to the user's objective."]
  ]
}

Past results  
[
  { "step": "Use ask_staff_expert to find the name of the department head.", "result": "Execution Error: The tool does not provide leadership information." }
]

New plan produced by you  
{
  "steps": [
    ["Search the official FAMNIT website for the Computer Science Department page and extract the full name of the department head."],
    ["Using that name, query ask_staff_expert for the email address of the department head and return it."],
    ["Synthesize all gathered information and provide the final comprehensive answer to the user's objective."]
  ]
}

## Example C: Summarization step only
Past results  
[
  { "step": "Retrieve the email for FAMNIT's international office.", "result": "international.office@famnit.upr.si" }
]

Original remaining plan  
{
  "steps": [
    ["Synthesize the gathered information and provide the final answer to the user's objective."]
  ]
}

New plan produced by you  
{
  "steps": []
}

## Example D: Walk-through of substitution
Objective What is the population of Ljubljana according to the 2021 census, and which source reports it?  

Original plan  
{
  "steps": [
    ["Find the population of Ljubljana from the 2021 census and return the number."],
    ["Using that number, locate the official statistical source and return its name."],
    ["Synthesize the gathered information and provide the final answer to the user's objective."]
  ]
}

Past results  
[
  { "step": "Find the population of Ljubljana from the 2021 census and return the number.", "result": "295 504" }
]

New plan produced by you  
{
  "steps": [
    ["Locate the official statistical source that reports a 2021 census population of 295 504 for Ljubljana and return the source’s full name."],
    ["Synthesize all gathered information and provide the final comprehensive answer to the user's objective."]
  ]
}

## Example E: Substituting a link from a previous step
Objective Provide the publication year of the paper “Attention Is All You Need” and cite its PDF link.  

Original plan  
{
  "steps": [
    ["Search for the paper 'Attention Is All You Need' and return the PDF link."],
    ["Using that link, open the PDF and extract the publication year."],
    ["Synthesize the gathered information and provide the final answer including the link to the user."]
  ]
}

Past results  
[
  { "step": "Search for the paper 'Attention Is All You Need' and return the PDF link.", "result": "https://arxiv.org/pdf/1706.03762.pdf" }
]

New plan produced by you  
{
  "steps": [
    ["Open the PDF at https://arxiv.org/pdf/1706.03762.pdf and extract the publication year (visible on the title page) and return it."],
    ["Synthesize all gathered information and provide the final comprehensive answer to the user's objective, including the link https://arxiv.org/pdf/1706.03762.pdf."]
  ]
}

## Example F: Completed objective
Original plan  
{
  "steps": [
    ["Use rag_lookup to find the email for the international student office at FAMNIT (use k=2)."],
    ["Synthesize the gathered information and provide the final answer including the link to the user."]
  ]
}

Past steps & results  
[
  { "step": "Use rag_lookup to find the email for the international student office at FAMNIT (use k=2).", "result": "The email for the international office is international.office@famnit.upr.si." }
]

Correct new JSON plan output  
{
  "steps": []
}

"#;

    let template = Template::simple(r#"
    in the future theese will be the tools avalible to you: 

    {{tools}}

    # Your original objective(user's task to complete) was: 

    {{prompt}}
    
    # Your original plan was: 
    
    {{plan}}
    
    
    # You have already completed the following steps and observed their results:
    
    {{past_steps}}

    "#);

    StatelessPrebuild::reply_without_tools()
        .import_ollama_config(ollama_config)
        .import_model_config(model_config)
        .import_prompt_config(prompt_config)
        .set_name("Plan revisor")
        .set_system_prompt(system_prompt)
        .set_template(template)
        .set_response_format(r#"
        {
            "type": "object",
            "properties": {
                "steps": {
                    "type": "array",
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        }
                    }
                }
            },
            "required": ["steps"]
        }
        "#)
        .set_model("hf.co/unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF:UD-Q4_K_XL")
        .set_clear_history_on_invocation(true)
        .build_with_notification()
        .await
}



pub async fn create_single_task_agent(ref_agent: &Agent) -> Result<(Agent, Receiver<Notification>), AgentBuildError> {
    let ollama_config = ref_agent.export_ollama_config();
    let model_config = ref_agent.export_model_config();
    let prompt_config = if let Ok(c) = ref_agent.export_prompt_config().await {
        c
    } else {
        PromptConfig::default()
    };

    let system_prompt = r#"You are the **Executor Agent**.  
You will receive **one inner array of step instructions** (all meant to run in parallel).  
A catalogue of callable tools is provided below the conversation.

# Execution protocol
1. **Tool phase (mandatory)** – In your first reply, call every function required to gather the data needed for *all* the given steps.  
   • Combine calls intelligently so everything can be retrieved in this single reply.  
   • Do not add commentary; your message should consist solely of the function calls.  
2. **Answer phase** – After the tool responses arrive, send a second reply that fulfils every step.

# Answer-phase requirements
* Your answer must be **exhaustive yet only include information actually returned by the tools**; do not invent facts.  
* Cite each sourced fact immediately after it, using a numbered inline citation: `[1](http://example.com)`.  
* Finish with a `## References` section listing every URL, numbered in order of first appearance.

*Citation example*

# Answer

The programme coordinator is Dr. Jane Doe [1](http://example.com/dr-jane-doe).  
Admission requires a completed bachelor’s degree [2](http://example.com/admission-requirements).

## References  
[1] http://example.com/dr-jane-doe  
[2] http://example.com/admission-requirements

* Include **all** relevant links you uncovered; every link must originate from the tool outputs shown in the conversation.  
* Respond in valid Markdown exactly as illustrated above.


    "#;

    StatelessPrebuild::call_tools_and_reply()
        .import_ollama_config(ollama_config)
        .import_model_config(model_config)
        .import_prompt_config(prompt_config)
        // .set_model("qwen3:8b")
        .set_model("hf.co/unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF:UD-Q4_K_XL")
        .set_name("Step executor")
        .set_system_prompt(system_prompt)
        .set_clear_history_on_invocation(true)
        .build_with_notification()
        .await
}

