use reagent::{error::AgentBuildError, prebuilds::StatelessPrebuild, util::Template, Agent, Notification};
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;
use tokio::sync::mpsc::Receiver;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Plan {
  pub steps: Vec<Vec<String>>,
}

pub async fn create_planner_agent(ref_agent: &Agent) -> Result<(Agent, Receiver<Notification>), AgentBuildError> {
    let ollama_config = ref_agent.export_ollama_config();
    let model_config = ref_agent.export_model_config();
    let prompt_config = ref_agent
        .export_prompt_config()
        .await
        .unwrap_or_default();
    
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
        .set_response_format(serde_json::to_string_pretty(&schema_for!(Plan)).unwrap())
        .set_system_prompt(system_prompt)
        .set_model("hf.co/unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF:UD-Q4_K_XL")
        // .set_model("gemma3:270m")
        .set_template(template)
        .set_clear_history_on_invocation(true)
        .build_with_notification()
        .await
}

