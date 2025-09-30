use std::{collections::BTreeMap, env};

use reagent_rs::{Agent, AgentBuildError, Notification, StatelessPrebuild, Template};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Receiver;

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct Requirement {
    pub function_usage_required: bool,
    pub recommended_params: Option<FreeObject>,
}

type FreeObject = BTreeMap<String, serde_json::Value>;

pub async fn build_function_filter_agent(
    urska: &mut Agent,
) -> Result<(Agent, Receiver<Notification>), AgentBuildError> {
    let system_prompt = r#"
    You will be given a user query and a tool with its JSON Schema for parameters.
    Decide if the tool should be used for the query.
    Derive usefullness from the description of the tool. If the tool might be useful,
    you should use it.
    If yes, produce the parameters object that conforms to the given schema.

    Rules
    1. Output a single JSON object that matches this schema exactly:
       {
         "function_usage_required": true|false,
         "recommended_params": <object>|null
       }
    2. If function_usage_required is false, set recommended_params to null.
    3. If true, recommended_params must be a valid JSON object that fits the tool's parameter schema.
    4. No explanations, no markdown fences, no extra keys, no comments.
    "#;

    let template = Template::simple(
        "\
    # Tool
    {{function}}

    # User query
    {{question}}",
    );

    StatelessPrebuild::reply_without_tools()
        .set_name("Source Filter")
        // .set_model(env::var("MODEL").expect("MODEL not set"))
        .set_base_url(env::var("OLLAMA_ENDPOINT").expect("OLLAMA_ENDPOINT not set"))
        .import_client_config(urska.export_client_config())
        .import_model_config(urska.export_model_config())
        .import_prompt_config(urska.export_prompt_config().await.unwrap_or_default())
        .set_system_prompt(system_prompt)
        .set_template(template)
        .set_response_format_from::<Requirement>()
        .build_with_notification()
        .await
}
