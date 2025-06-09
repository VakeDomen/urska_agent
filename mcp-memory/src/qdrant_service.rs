use std::fmt;
use std::sync::Arc;

use qdrant_client::prelude::{Payload, QdrantClient}; 
use qdrant_client::qdrant::point_id::PointIdOptions;
use qdrant_client::qdrant::{
    vectors_config, CreateCollection, Distance, PointId, PointStruct, SearchPoints, VectorParams,
    VectorsConfig, // Keep Value if directly constructing qdrant::Value, not strictly needed here with serde_json conversion
};
use crate::config::Config;
use crate::models::{MemoryItem, QueryResultItem};
use anyhow::Result;
use chrono::{DateTime, Utc}; // Ensure DateTime and Utc are imported if used here
use uuid::Uuid; // Ensure Uuid is imported

#[derive(Clone)]
pub struct QdrantService {
    client: Arc<QdrantClient>,
    collection_name: String,
}

impl fmt::Debug for QdrantService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QdrantService")
            .field("collection_name", &self.collection_name)
            .finish()
    }
}


impl QdrantService {
    pub async fn new(config: &Config) -> Result<Self> {
        // Simplified client initialization as per qdrant-client common usage
        let client = QdrantClient::from_url(&config.qdrant_endpoint).build()?;
        
        println!("Attempting to connect to Qdrant at: {}", config.qdrant_endpoint);

        // Ensure collection exists
        // It's often good practice to check if the client can connect before proceeding
        match client.list_collections().await {
            Ok(collections_list) => {
                if !collections_list
                    .collections
                    .iter()
                    .any(|c| c.name == config.qdrant_collection_name)
                {
                    println!(
                        "Collection '{}' not found. Attempting to create it.",
                        config.qdrant_collection_name
                    );
                    client
                        .create_collection(&CreateCollection {
                            collection_name: config.qdrant_collection_name.clone(),
                            vectors_config: Some(VectorsConfig {
                                config: Some(vectors_config::Config::Params(VectorParams {
                                    size: config.embedding_dimension,
                                    distance: Distance::Cosine.into(),
                                    ..Default::default()
                                })),
                            }),
                            ..Default::default()
                        })
                        .await?;
                    println!("Created Qdrant collection: {}", config.qdrant_collection_name);
                } else {
                    println!(
                        "Qdrant collection '{}' already exists.",
                        config.qdrant_collection_name
                    );
                }
            }
            Err(e) => {
                println!("Failed to connect to Qdrant or list collections: {}", e);
                return Err(anyhow::anyhow!(
                    "Failed to connect to Qdrant or list collections: {}",
                    e
                ));
            }
        }

        Ok(Self {
            client: client.into(),
            collection_name: config.qdrant_collection_name.clone(),
        })
    }

    pub async fn add_memory(&self, memory_item: &MemoryItem) -> Result<()> {
        let embedding = memory_item
            .embedding
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Embedding not found for memory item"))?;

        // Ensure metadata is included in the payload
        let payload_json = serde_json::json!({
            "text": memory_item.text,
            "timestamp": memory_item.timestamp.to_rfc3339(),
        });

        let payload: Payload = payload_json
            .try_into()
            .map_err(|e| anyhow::anyhow!("Failed to convert JSON to Qdrant Payload: {}", e))?;

        let point_id: PointId = memory_item.id.to_string().into(); // Convert UUID string to PointId

        let point = PointStruct::new(point_id, embedding.clone(), payload);
        
        // Using upsert_points which is async. upsert_points_blocking is also async,
        // the naming can be a bit confusing but both are suitable for async contexts.
        // upsert_points is generally fine.
        self.client
            .upsert_points(self.collection_name.clone(), None, vec![point], None)
            .await?;
        println!("Upserted point {} to collection '{}'", memory_item.id, self.collection_name);
        Ok(())
    }

    pub async fn search_memories(
        &self,
        query_embedding: Vec<f32>,
        top_k: usize,
    ) -> Result<Vec<QueryResultItem>> {
        let search_request = SearchPoints {
            collection_name: self.collection_name.clone(),
            vector: query_embedding,
            limit: top_k as u64,
            with_payload: Some(true.into()),
            ..Default::default()
        };

        let search_result = self.client.search_points(&search_request).await?;
        println!(
            "Search in collection '{}' returned {} results",
            self.collection_name,
            search_result.result.len()
        );

        let results = search_result
            .result
            .into_iter()
            .map(|scored_point| {
                let id_str = scored_point.id.map_or_else(
                    || {
                        println!("Scored point missing id, generating new UUID.");
                        Uuid::new_v4().to_string()
                    },
                    |id_val| match id_val.point_id_options {
                        Some(PointIdOptions::Uuid(s)) => s,
                        Some(PointIdOptions::Num(n)) => n.to_string(),
                        None => {
                            println!("Scored point missing id_options, generating new UUID.");
                            Uuid::new_v4().to_string()
                        }
                    },
                );

                let payload_map = scored_point.payload;
                let text = payload_map
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap() // Provide a default if text is missing or not a string
                    .to_string();

                let timestamp_str = payload_map
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .unwrap(); // Provide a default if timestamp is missing

                let timestamp =
                    DateTime::parse_from_rfc3339(timestamp_str)
                        .map(DateTime::<Utc>::from)
                        .unwrap_or_else(|e| {
                            println!(
                                "Failed to parse timestamp '{}': {}. Falling back to Utc::now().",
                                timestamp_str, e
                            );
                            Utc::now()
                        });
                
                // Ignoring metadata conversion as requested
                let metadata_serde_json = payload_map.get("metadata").map_or(serde_json::Value::Null, |v| {
                    // If you were to implement it, it would be:
                    // convert_qdrant_value_to_serde_json(v.clone())
                    serde_json::Value::Null // Placeholder for ignored metadata
                });


                QueryResultItem {
                    id: id_str,
                    text,
                    timestamp,
                    score: scored_point.score,
                    metadata: metadata_serde_json, // Will be Null due to simplification
                }
            })
            .collect();

        Ok(results)
    }
}
