use qdrant_client::qdrant::ScoredPoint;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct ResultChunk {
    pub id: String,
    pub question: String,
    pub document_id: String,
    pub chunk: String,
    pub seq_num: i32,
    pub document_name: String,
    pub score: f32,
}

impl From<ScoredPoint> for ResultChunk {
    fn from(value: ScoredPoint) -> Self {
        let id: String = match value.id {
            Some(d) => format!("{:?}", d),
            None => "Unknown".into(),
        };

        let question = match value.payload.get("question") {
            Some(d) => d.as_str().map_or("Unknown", |v| v).to_owned(),
            None => "Unknown".to_owned(),
        };
        
        let document_id = match value.payload.get("document_id") {
            Some(d) => d.as_str().map_or("Unknown", |v| v).to_owned(),
            None => "Unknown".to_owned(),
        };

        
        let chunk = match value.payload.get("chunk") {
            Some(d) => d.as_str().map_or("Unknown", |v| v).to_owned(),
            None => "Unknown".to_owned(),
        };

        let seq_num = match value.payload.get("seq_num") {
            Some(d) => d.as_integer().unwrap_or(-1) as i32,
            None => -1,
        };

        let document_name = match value.payload.get("document_name") {
            Some(d) => d.as_str().map_or("Unknown", |v| v)
                .to_owned()
                .replace("_", "/")
                .replace(".md", ""),
            None => "Unknown".to_owned(),
        };

        Self {
            id,
            question,
            document_id,
            chunk,
            seq_num,
            document_name,
            score: value.score,
        }
    }
}

impl Into<String> for &ResultChunk {
    fn into(self) -> String {
        format!(r#"
            ---
            Source: {}

            Passage content: 
            
            {}
            
            ---

            "#,
            self.document_name,
            self.chunk
        )
    }
}