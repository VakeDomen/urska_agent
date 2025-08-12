use std::{fs::File, io::{BufWriter, Write}};

use anyhow::{anyhow, Result};
use comm::{
    embedding::EmbeddingVector,
    qdrant::{insert_chunks_to_qdrant, vector_search},
    OllamaClient,
};
use loading::load_file;
use models::SearchResult;
use ollama_rs::generation::embeddings::request::{EmbeddingsInput, GenerateEmbeddingsRequest};
use processing::{chunk, dedup, hype, prepare_for_upload, prompt};

pub mod comm;
pub mod loading;
pub mod models;
pub mod processing;

pub use models::RagProcessableFile;

use crate::rag::{comm::qdrant::vector_search_k, models::chunks::ResultChunk};

#[derive(Debug, Default)]
pub struct Rag {
    ollama: OllamaClient,
}

impl Rag {
    pub async fn insert(&self, file: RagProcessableFile) -> Result<()> {
        let loaded_file = load_file(&file)?;
        let chunked_file = chunk(loaded_file, processing::ChunkingStrategy::Word(250, 30));
        let enriched_file = hype(chunked_file, &self.ollama).await;
        let embedded_chunks = prepare_for_upload(enriched_file, &self.ollama).await?;
        insert_chunks_to_qdrant(embedded_chunks).await
    }

    pub async fn insert_with_strategy(&self, file: RagProcessableFile, strategy: processing::ChunkingStrategy) -> Result<()> {
        let name = file.original_name.clone();
        let loaded_file = load_file(&file)?;
        let chunked_file = chunk(loaded_file, strategy);
        let enriched_file = hype(chunked_file, &self.ollama).await;

        let file = File::create(&format!("./resources/uploaded/result_{}.json", name))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &enriched_file)?;
        writer.flush()?;

        let embedded_chunks = prepare_for_upload(enriched_file, &self.ollama).await?;
        insert_chunks_to_qdrant(embedded_chunks).await
    }

    pub async fn search(&self, query: String) -> Result<SearchResult> {
        let emb_query = GenerateEmbeddingsRequest::new("bge-m3".to_owned(), EmbeddingsInput::Single(query.clone()));
        let embedding = match self.ollama.embed(emb_query).await {
            Ok(resp) => EmbeddingVector(resp.embeddings[0].clone()),
            Err(e) => return Err(anyhow!(format!("Failed embedding the query: {}", e))),
        };
        let resp = vector_search(embedding).await?;
        let resp = dedup(resp);
        println!("{:#?}", resp);
        match prompt(query, resp, &self.ollama).await {
            Ok(r) => Ok(r),
            Err(e) => Err(anyhow!(e.to_string())),
        }
    }

    pub async fn search_k(&self, query: String, k: u64) -> Result<Vec<ResultChunk>> {
        let emb_query = GenerateEmbeddingsRequest::new("bge-m3".to_owned(), EmbeddingsInput::Single(query.clone()));
        let embedding = match self.ollama.embed(emb_query).await {
            Ok(resp) => EmbeddingVector(resp.embeddings[0].clone()),
            Err(e) => return Err(anyhow!(format!("Failed embedding the query: {}", e))),
        };
        let resp = vector_search_k(embedding, k).await?;
        println!("HITS: {:#?}", resp);
        Ok(dedup(resp))
    }
}
