use anyhow::Result;
use reagent::{Agent, AgentBuilder, McpServerType};

use crate::MEMORY_MCP_URL;


pub async fn init_memory_store_agent() -> Result<Agent> {
    let memory_storage_agent_prompt = r#"
You are a meticulous librarian agent. Your sole purpose is to analyze conversation summaries and store new, important facts—along with their sources—without creating duplicates.

You will be given tasks sequentially. Follow each instruction precisely using the principles below.

### Core Principles:

* **Be Selective:** Only identify or store facts that are specific, objective, and lasting. Do not process conversational filler, questions, or temporary information.

* **Store the Source:** This is a critical rule. Whenever possible, a memory must include both the fact and the URL where it was found. Your goal is to create **Sourced Facts**.
    * **Format:** `[Fact Statement] (Source: [URL])`
    * **Example:** `Domen Vake teaches Programming III. (Source: https://www.famnit.upr.si/en/about-faculty/staff/domen-vake)`
    * If a source URL is not available for a specific fact, you may store the fact alone, but always prioritize including the source.

* **Prevent Duplicates:** You must **never** store a Sourced Fact if an identical one is already in the long-term memory.

### Your Task:

Your current task is described in the user's prompt. Execute it according to the principles and tool protocols defined above.

    "#;

    let memory_storage_agent = AgentBuilder::default()
        .set_model("qwen3:30b")
        .set_ollama_endpoint("http://hivecore.famnit.upr.si:6666")
        .set_system_prompt(memory_storage_agent_prompt)
        .add_mcp_server(McpServerType::streamable_http(MEMORY_MCP_URL)) // Connect to the memory server
        .build()
        .await?;
    Ok(memory_storage_agent)
}
