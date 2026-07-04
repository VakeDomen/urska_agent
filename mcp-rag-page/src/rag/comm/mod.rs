use ollama_rs::{
    error::OllamaError,
    generation::{
        completion::{GenerationResponse, GenerationResponseStream},
        embeddings::{request::GenerateEmbeddingsRequest, GenerateEmbeddingsResponse},
    },
    Ollama,
};
use question::Question;
use std::env;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

pub mod embedding;
pub mod qdrant;
pub mod question;

#[derive(Debug)]
pub struct OllamaClient {
    ollama: Ollama,
}

impl Default for OllamaClient {
    fn default() -> Self {
        let ollama_host = env::var("OLLAMA_HOST").expect("OLLAMA HOST not set");
        let ollama_port = env::var("OLLAMA_PORT").expect("OLLAMA PORT not set");
        let ollama_port: u16 = ollama_port.parse().expect("OLLAMA_PORT not u16");
        let ollama_key = env::var("OLLAMA_KEY").expect("OLLAMA_KEY not set");

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", ollama_key))
                .expect("invalid header value"),
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .expect("failed to build reqwest client");

        Self {
            ollama: Ollama::new_with_client(ollama_host, ollama_port, client),
        }
    }
}

impl OllamaClient {
    pub async fn generate(&self, question: Question) -> Result<GenerationResponse, OllamaError> {
        self.ollama.generate((&question).into()).await
    }

    pub async fn generate_stream(&self, question: Question) -> Result<GenerationResponseStream, OllamaError> {
        self.ollama.generate_stream((&question).into()).await
    }

    pub async fn embed(&self, req: GenerateEmbeddingsRequest) -> Result<GenerateEmbeddingsResponse, OllamaError> {
        self.ollama.generate_embeddings(req).await
    }

    pub async fn answer_all(&self, questions: Vec<Question>) -> Vec<String> {
        let futures = questions.into_iter().map(|q| async move { self.generate(q.clone()).await.ok() });

        let results = futures::future::join_all(futures).await;
        results
            .into_iter()
            .map(|r| r.map_or_else(|| "".to_owned(), |resp| resp.response))
            .collect()
    }
}
