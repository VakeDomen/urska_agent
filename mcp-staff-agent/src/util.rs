use std::collections::HashMap;

use reagent::{Message, Role};

pub fn rank_names(mut names: Vec<String>, query: &str) -> Vec<String> {
    // Pre-compute the query vector once
    let q_vec = trigram_vec(&query.to_lowercase());

    names.sort_by(|a, b| {
        let sim_a = cosine_sim(&trigram_vec(&a.to_lowercase()), &q_vec);
        let sim_b = cosine_sim(&trigram_vec(&b.to_lowercase()), &q_vec);
        // higher similarity ⇒ earlier in list
        sim_b
            .partial_cmp(&sim_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    names
}

/// Build a (trigram → frequency) sparse vector.
fn trigram_vec(s: &str) -> HashMap<String, usize> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 3 {
        // For very short strings use the whole string as one “token”
        return HashMap::from([(s.to_string(), 1)]);
    }

    let mut v = HashMap::new();
    for window in chars.windows(3) {
        let tri: String = window.iter().collect();
        *v.entry(tri).or_insert(0) += 1;
    }
    v
}

/// Cosine similarity between two sparse vectors.
fn cosine_sim(a: &HashMap<String, usize>, b: &HashMap<String, usize>) -> f64 {
    let dot: usize = a
        .iter()
        .filter_map(|(k, &va)| b.get(k).map(|&vb| va * vb))
        .sum();

    let norm = |v: &HashMap<String, usize>| {
        (v.values().map(|&x| (x * x) as f64).sum::<f64>()).sqrt()
    };

    let denom = norm(a) * norm(b);
    if denom == 0.0 {
        0.0
    } else {
        dot as f64 / denom
    }
}


/// New helper function to serialize a conversation history into a single prompt for the memory agent.
pub fn history_to_memory_prompt(history: Vec<Message>) -> String {
    let mut prompt = String::from("Here is a summary of a conversation. Please analyze it for new facts to store in memory:\n\n---\n\n");
    for msg in history.iter().skip(2) { // Skip the system prompt and the initial memory query result
        let content = msg.content.clone().unwrap_or_default();
        match msg.role {
            Role::User => prompt.push_str(&format!("USER ASKED: {}\n\n", content)),
            Role::Assistant => prompt.push_str(&format!("ASSISTANT: {}\n\n", content)),
            Role::Tool => {
                let tool_name = msg.tool_call_id.as_deref().unwrap_or("unknown_tool");
                prompt.push_str(&format!("TOOL `{:?}` RETURNED:\n{}\n\n", tool_name, content));
            }
            Role::System => continue,
        }
    }
    prompt.push_str("---\nEnd of conversation summary.");

    println!("CONVO: {}", prompt);
    prompt
}
