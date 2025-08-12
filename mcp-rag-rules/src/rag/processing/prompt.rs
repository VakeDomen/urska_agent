use crate::rag::{
    comm::{question::Question, OllamaClient},
    models::{chunks::ResultChunk, SearchResult},
};
use ollama_rs::{error::OllamaError, generation::completion::GenerationResponseStream};

pub async fn prompt(prompt: String, chunks: Vec<ResultChunk>, ollama: &OllamaClient) -> Result<SearchResult, OllamaError> {
    let llm_prompt = construct_prompt(prompt, &chunks);
    println!("Prompt: {:#?}", llm_prompt);
    let stream: GenerationResponseStream = ollama.generate_stream(llm_prompt).await?;
    Ok(SearchResult { chunks, stream })
}

fn construct_prompt(prompt: String, chunks: &Vec<ResultChunk>) -> Question {
    let system_message = "/no_think You are an assistant who is helping with finding information \
        in the repository of information. You are a guide through the documents. Given a \
        question, help navigate through the files and the information. You are allowed to read \
        some of the documents: "
        .to_string();

    let context: Vec<String> = chunks.iter().map(|c| c.into()).collect();

    let question = format!(r#"
    
    {}
    
    User question to answe based on above data:
    {}
    
    Notes: 
    - Respond in markdown

    "#, context.join("\n"), prompt);

    Question::from(question).set_system_prompt(&system_message)
}
