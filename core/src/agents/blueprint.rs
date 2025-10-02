use std::env;

use reagent_rs::{Agent, AgentBuildError, Notification, StatelessPrebuild, Template};
use tokio::sync::mpsc::Receiver;

pub async fn create_blueprint_agent(
    ref_agent: &Agent,
) -> Result<(Agent, Receiver<Notification>), AgentBuildError> {
    let ollama_config = ref_agent.export_client_config();
    let model_config = ref_agent.export_model_config();
    let prompt_config = ref_agent.export_prompt_config().await.unwrap_or_default();

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

    FAQ:
    - you are given acces to simialar FAQ questions
    - if the answer can be derived from the FAQ only plan to retrieve accompanying resources
    - if the answer can be derived from the FAQ prioratize short plans
    "#;

    let template = Template::simple(
        r#"
    # These tools will later be avalible to the executor agent:

    {{tools}}

    FAQ:

    {{faq}}

    Users task to create a blueprint for:

    {{prompt}}
    "#,
    );

    StatelessPrebuild::reply_without_tools()
        .import_client_config(ollama_config)
        .import_model_config(model_config)
        .import_prompt_config(prompt_config)
        .set_name("Thinker")
        .set_base_url(env::var("OLLAMA_ENDPOINT").expect("OLLAMA_ENDPOINT not set"))
        .set_model(env::var("MODEL").expect("MODEL not set"))
        .set_system_prompt(system_prompt)
        .set_template(template)
        .set_clear_history_on_invocation(true)
        .build_with_notification()
        .await
}
