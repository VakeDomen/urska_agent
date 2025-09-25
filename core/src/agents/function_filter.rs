use std::env;

use reagent_rs::{Agent, AgentBuildError, Notification, StatelessPrebuild, Template, ToolCall, ToolCallFunction, ToolType};
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;
use tokio::sync::mpsc::Receiver;


#[derive(Debug, JsonSchema, Deserialize)]
pub struct Requirement {
    pub function_usage_required: bool,
}

pub async fn build_function_filter_agent(urska: &mut Agent) -> Result<(Agent, Receiver<Notification>), AgentBuildError> {
    let system_prompt = r#"
    You will be given a prompt and a function.
    Your task is to asses wether the useage of the given function is appropriate to answer the user query.
    "#;

    let template = Template::simple("# function to assess: \n\n {{function}}\n\n# User query:\n\n{{question}}");

    StatelessPrebuild::reply_without_tools()
        .set_name("Source Filter")
        .set_base_url(env::var("OLLAMA_ENDPOINT").expect("OLLAMA_ENDPOINT not set"))
        .set_model(env::var("MODEL").expect("MODEL not set"))
        .import_client_config(urska.export_client_config())
        .import_model_config(urska.export_model_config())
        .import_prompt_config(urska.export_prompt_config().await.unwrap_or_default())
        .set_system_prompt(system_prompt)
        .set_template(template)
        .set_response_format_from::<Requirement>()
        .build_with_notification()
        .await
}

