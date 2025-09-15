use reagent_rs::{flow, invoke_with_tool_calls, Agent, AgentBuildError, AgentBuilder, AgentError, Message, Notification, NotificationContent};
use tokio::sync::mpsc::Receiver;

pub async fn create_single_task_agent(ref_agent: &Agent) -> Result<(Agent, Receiver<Notification>), AgentBuildError> {
    let ollama_config = ref_agent.export_client_config();
    let model_config = ref_agent.export_model_config();
    let prompt_config = ref_agent
        .export_prompt_config()
        .await
        .unwrap_or_default();

    let system_prompt = r#"
You are the Executor Agent.
You will be given a single step from a plan to execute.
You have access to a variety of tools to help you complete the step.
Your goal is to complete the step as efficiently as possible, using tools as needed, and then provide a final answer that addresses the user's original objective.

# Important rules
Tool catalogue
A catalogue of callable tools appears below the conversation. Use only those tools. Do not invent tools or URLs.

Execution protocol
1. Tool phase
   • Your first reply must contain only the function call(s) needed to execute this single step.
   • If this step depends on links discovered earlier, read the conversation history and call tools accordingly.
   • If you do not yet have at least one usable source URL for every fact you will state, issue additional tool-only replies to obtain sources. For upr.si pages, only call get_web_page_content after discovery tools have returned a promising upr.si link.

2. Answer phase
   • After tool responses arrive and you have sufficient sources, send one Markdown reply that fulfils this step.

Answer-phase requirements
• Use only information actually returned by tools in this conversation. Do not fabricate or guess.
• Cite each fact immediately after it with a numbered inline citation: [1](https://example.com).
• End with a “## References” section listing every URL in order of first appearance, one per line with its number.
• Include all relevant links you uncovered. Every link must originate from tool outputs in this conversation.
• Respond in valid Markdown exactly as illustrated. It is extremely important all links are correctly written.

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

# Answer

The programme coordinator is Dr. Jane Doe [1](http://example.com/dr-jane-doe).
Admission requires a completed bachelor’s degree [2](http://example.com/admission-requirements).

## References
[1] http://example.com/dr-jane-doe
[2] http://example.com/admission-requirements

    skip and DO NOT include references if there is no relevant links. 
    Never use example.com or similar placeholders.


    "#;

    AgentBuilder::default()
        .import_client_config(ollama_config)
        .import_model_config(model_config)
        .import_prompt_config(prompt_config)
        .set_name("Step executor")
        .set_model("hf.co/unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF:UD-Q4_K_XL")
        .strip_thinking(true)
        // .set_model("qwen3:0.6b")
        // .set_model("gemma3:270m")
        .set_system_prompt(system_prompt)
        .set_flow(flow!(executor_flow))
        // .set_clear_history_on_invocation(true)
        .set_max_iterations(15)
        .build_with_notification()
        .await
}


async fn executor_flow(agent: &mut Agent, prompt: String) -> Result<Message, AgentError> {
    agent.history.push(Message::user(prompt));
    
    let mut resp = invoke_with_tool_calls(agent).await?;
    for _ in 0..agent.max_iterations.unwrap_or(5) {
    
        if resp.message.tool_calls.is_none() {
            break;
        }
        resp = invoke_with_tool_calls(agent).await?;
    } 
    // let response = invoke_without_tools(agent).await?;
    agent.notify(NotificationContent::Done(true, resp.message.content.clone())).await;
    Ok(resp.message)
}
