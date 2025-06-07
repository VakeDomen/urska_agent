
use std::{error::Error, sync::Arc};
use tokio::sync::Mutex;
use reagent::{AgentBuilder, AsyncToolFn, McpServerType, ToolBuilder, ToolExecutionError};
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let weather_agent = AgentBuilder::default()
        .set_model("qwen3:30b")
        .set_ollama_endpoint("http://hivecore.famnit.upr.si")
        .set_ollama_port(6666)
        .set_system_prompt("/no_think \nYou make up weather info in JSON. You always say it's snowing.")
        .set_response_format(
            r#"
            {
              "type":"object",
              "properties":{
                "windy":{"type":"boolean"},
                "temperature":{"type":"integer"},
                "description":{"type":"string"}
              },
              "required":["windy","temperature","description"]
            }
            "#,
        )
        .build()
        .await?;

    let weather_ref = Arc::new(Mutex::new(weather_agent));
    let weather_exec: AsyncToolFn = {
        let weather_ref = weather_ref.clone();
        Arc::new(move |args: Value| {
            let weather_ref = weather_ref.clone();
            Box::pin(async move {
                let mut agent = weather_ref.lock().await;
                
                let loc = args.get("location")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolExecutionError::ArgumentParsingError("Missing 'location' argument".into()))?;

                let prompt = format!("/no_think What is the weather in {}?", loc);

                let resp = agent.invoke(prompt)
                    .await
                    .map_err(|e| ToolExecutionError::ExecutionFailed(e.to_string()))?;
                Ok(resp.content.unwrap_or_default())
            })
        })
    };

    let weather_tool = ToolBuilder::new()
        .function_name("get_current_weather")
        .function_description("Returns a weather forecast for a given location")
        .add_property("location", "string", "City name")
        .add_required_property("location")
        .executor(weather_exec)
        .build()?;


    let agent_system_prompt = r#"
    You are a helpful assistant. Your primary goal is to provide accurate, relevant, and 
    context-aware responses by effectively managing and utilizing your memory.

    **Your Operational Protocol:**

    1.  **Understand User & Recall from Memory:**
        * Carefully analyze the user's current query.
        * Before formulating a response, consider if information from your long-term 
        memory could be relevant.
        * If you suspect relevant information might exist (e.g., related to past topics, 
        user preferences, or facts previously discussed), **use the `query_memory` tool**.
            * Formulate a concise `query_text` for the `query_memory` tool that captures 
            the essence of what you're trying to recall. For example, if the user asks 
            "What was that project I mentioned?", you might use `query_memory` with 
            `query_text: "user's mentioned project"`.
            * Review the results from `query_memory` to inform your response.

    2.  **Identify New Key Information for Long-Term Storage:**
        * As you process the user's query and prepare your response, critically assess 
        if the current interaction contains **new, core information** that would be 
        genuinely useful for maintaining context, understanding preferences, or improving 
        the quality of future interactions.
        * **Examples of information to consider saving with `add_memory`:**
            * Explicit user preferences (e.g., "I prefer brief answers," "My name is Alex,"
             "My favorite color is blue", "it's raining").
            * Important facts or constraints stated by the user (e.g., "I'm working on a 
            project about climate change," "The deadline is next Friday," "My current 
            location is Koper").
            * Key decisions made during the conversation.
            * The central topic or goal if it's newly established or significantly shifts.
        * **Do NOT save with `add_memory`:** Trivial details, information that is clearly 
        transient, redundant information already likely captured effectively by 
        `query_memory` from past saves, or every single piece of user input. Focus on 
        the *value for future recall* of *newly established* important points.

    3.  **Save to Memory (Action Step):**
        * If you identify such **new key information** according to step 2, you **MUST 
        use the `add_memory` tool** to store a concise summary or the direct piece of 
        information as `text`.
        * This `add_memory` action should typically occur *before* you provide your 
        final answer to the user for the current query.
        * *Example:* If the user says "My company is called 'Innovatech Solutions'", you 
        should use `add_memory` with `text: "User's company: Innovatech Solutions"`.

    4.  **Formulate & Deliver Response:**
        * Once you have completed any necessary memory operations (using `query_memory` 
        for recall and `add_memory` for saving new information), formulate and provide your 
        answer to the user's current query.
        * If you used `get_current_weather`, incorporate its output into your response.

    Remember to prioritize helpfulness and relevance, leveraging your memory tools 
    effectively. Always consider if querying existing memory or adding new information will
    improve the current or future conversation."#;
    
    let stop_prompt = r#"
    **Decision Point:**

    Your previous output is noted. Now, explicitly decide your next step:

    1.  **Continue Working:** If you need to perform more actions (like calling a tool such 
    as `add_memory`, `get_current_weather`, `query_memory`, or doing more internal 
    reasoning), clearly state your next specific action or internal thought. **Do NOT use 
    `<final>` tags for this.**

    2.  **Final Answer:** If you completed all the tasks and want to submit the final answer 
    for the user, re-send that **entire message** now, but wrap it in `<final>Your final message 
    here</final>` tags.

    Choose one option.
    "#;

    let mut agent = AgentBuilder::default()
        .set_model("qwen3:30b")
        .set_ollama_endpoint("http://hivecore.famnit.upr.si")
        .set_ollama_port(6666)
        .set_system_prompt(agent_system_prompt)
        .add_mcp_server(McpServerType::sse("http://localhost:8001/sse"))
        .add_tool(weather_tool)
        .set_stopword("<final>")
        .set_stop_prompt(stop_prompt)
        .build()
        .await?;

    let resp = agent.invoke("Say hello").await?;
    println!("\n-> Agent: {}", resp.content.unwrap_or_default());

    let resp = agent.invoke("What is the current weather in Koper?").await?;
    println!("\n-> Agent: {}", resp.content.unwrap_or_default());

    let resp = agent.invoke("What do you remember?").await?;
    println!("\n-> Agent: {}", resp.content.unwrap_or_default());

    println!("Agent: {:#?}", agent);

    Ok(())
}
