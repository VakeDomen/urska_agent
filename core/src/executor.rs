use reagent::{error::AgentBuildError, flow_types::{Flow, FlowFuture}, invocations::{invoke_with_tool_calls, invoke_without_tools}, Agent, AgentBuilder, Message, Notification, NotificationContent};
use tokio::sync::mpsc::Receiver;

pub async fn create_single_task_agent(ref_agent: &Agent) -> Result<(Agent, Receiver<Notification>), AgentBuildError> {
    let ollama_config = ref_agent.export_ollama_config();
    let model_config = ref_agent.export_model_config();
    let prompt_config = ref_agent
        .export_prompt_config()
        .await
        .unwrap_or_default();

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
* Its extremely important all links are correctly written. 


    "#;

    AgentBuilder::default()
        .import_ollama_config(ollama_config)
        .import_model_config(model_config)
        .import_prompt_config(prompt_config)
        .set_name("Step executor")
        .set_model("hf.co/unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF:UD-Q4_K_XL")
        // .set_model("gemma3:270m")
        .set_system_prompt(system_prompt)
        .set_flow(Flow::Custom(executor_flow))
        .set_clear_history_on_invocation(true)
        .build_with_notification()
        .await
}


fn executor_flow<'a>(agent: &'a mut Agent, prompt: String) -> FlowFuture<'a> {
    Box::pin(async move {
        agent.history.push(Message::user(prompt));
        let _ = invoke_with_tool_calls(agent).await?;
        let response = invoke_without_tools(agent).await?;
        agent.notify(NotificationContent::Done(true, response.message.content.clone())).await;
        Ok(response.message)
    })   
}
