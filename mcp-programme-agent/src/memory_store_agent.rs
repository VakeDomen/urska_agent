use anyhow::Result;
use reagent::{Agent, AgentBuilder, McpServerType};

use crate::MEMORY_MCP_URL;


pub async fn init_memory_store_agent() -> Result<Agent> {
    let memory_storage_agent_prompt = r#"
    You are a meticulous librarian agent. Your sole purpose is to analyze conversation summaries and store new, important facts without creating duplicates.

    You will be given tasks sequentially. Follow each instruction precisely using the principles below.

    ### Core Principles:

    * **Be Selective:** Only identify or store facts that are specific, objective, and lasting (e.g., "Domen Vake teaches Programming III."). Do not process conversational filler, questions, or temporary information.
    * **Prevent Duplicates:** This is your most important rule. You must **never** store a fact if it is already in the long-term memory.

    ### Your Task:

    Your current task is described in the user's prompt. Execute it according to the principles and tool protocols defined above.

    "#;

    let memory_storage_agent = AgentBuilder::default()
        .set_model("qwen3:30b")
        .set_ollama_endpoint("http://hivecore.famnit.upr.si")
        .set_ollama_port(6666)
        .set_system_prompt(memory_storage_agent_prompt)
        .add_mcp_server(McpServerType::sse(MEMORY_MCP_URL)) // Connect to the memory server
        .build()
        .await?;
    Ok(memory_storage_agent)
}