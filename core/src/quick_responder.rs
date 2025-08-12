use reagent::{configs::PromptConfig, error::AgentBuildError, prebuilds::StatelessPrebuild, util::Template, Agent, Notification};
use serde::Deserialize;
use tokio::sync::mpsc::Receiver;
use schemars::{schema_for, JsonSchema};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Answerable {
    pub can_respond: bool
}

pub async fn create_quick_response_agent(ref_agent: &Agent) -> Result<(Agent, Receiver<Notification>), AgentBuildError> {
    let ollama_config = ref_agent.export_ollama_config();
    let model_config = ref_agent.export_model_config();
    let prompt_config = if let Ok(c) = ref_agent.export_prompt_config().await {
        c
    } else {
        PromptConfig::default()
    };
    
    let system_prompt = r#"
    Evaluate if the user prompt can be answered with the given FAQ. If the answer is not directly 
    extractable from the FAQ answer with false.
    "#;

    let template = Template::simple(r#"
    FAQ:

    {{faq}}

    ---

    The question to answer: 

    {{prompt}}


    Cant he above prompt be answered exhaustively?
    "#);


    StatelessPrebuild::reply_without_tools()
        .import_ollama_config(ollama_config)
        .import_model_config(model_config)
        .import_prompt_config(prompt_config)
        .set_name("Quick")
        // .set_model("hf.co/unsloth/Qwen3-4b-Thinking-2507-GGUF:UD-Q4_K_XL")
        .set_model("qwen3:0.6b")
        .set_template(template)
        .set_response_format(serde_json::to_string_pretty(&schema_for!(Answerable)).unwrap())
        .set_system_prompt(system_prompt)
        .set_clear_history_on_invocation(true)
        .build_with_notification()
        .await
}
