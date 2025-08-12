use qdrant_client::qdrant::{ScoredPoint, Value as QValue};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ResultChunk {
    pub id: String,
    pub thread_id: String,
    pub question: String,
    pub answer: String,
    pub keywords: Vec<String>,
    pub classification: String,
    pub score: f32,
}

impl From<ScoredPoint> for ResultChunk {
    fn from(value: ScoredPoint) -> Self {
        // id can be numeric or uuid in Qdrant, keep debug format like before
        let id: String = match value.id {
            Some(d) => format!("{:?}", d),
            None => "Unknown".into(),
        };

        let get_str = |key: &str| -> String {
            value
                .payload
                .get(key)
                .and_then(QValue::as_str)
                .unwrap_or(&"Unknown".to_string())
                .to_owned()
        };

        let thread_id = get_str("thread_id");
        let question = get_str("question");
        let answer = get_str("answer");
        let classification = get_str("classification");

        // keywords is a list of strings in your payload
        let keywords: Vec<String> = value
            .payload
            .get("keywords")
            .and_then(QValue::as_list)
            .map(|lst| {
                lst.iter()
                    .filter_map(QValue::as_str)
                    .map(|s| s.to_owned())
                    .collect::<Vec<_>>()
            })
            // fallback if someone accidentally indexed as a comma separated string
            .or_else(|| {
                let s = value.payload.get("keywords").and_then(QValue::as_str)?;
                let items = s
                    .split(',')
                    .map(|x| x.trim())
                    .filter(|x| !x.is_empty())
                    .map(|x| x.to_owned())
                    .collect::<Vec<_>>();
                Some(items)
            })
            .unwrap_or_default();

        Self {
            id,
            thread_id,
            question,
            answer,
            keywords,
            classification,
            score: value.score,
        }
    }
}

// Pretty rendering without using double hyphens in separators
impl Into<String> for &ResultChunk {
    fn into(self) -> String {
        format!(
            "\n---\n\nScore: {}\n\nKeywords: {}\n\nQuestion:\n{}\n\nAnswer:\n{}\n\n---\n",
            self.score,
            if self.keywords.is_empty() { String::from("(none)") } else { self.keywords.join(", ") },
            self.question,
            self.answer
        )
    }
}
