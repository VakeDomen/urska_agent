use std::path::PathBuf;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};


type Url = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RagProcessableFileType {
    Text,
    Markdown,
    Pdf,
    Html(Url)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagProcessableFile {
    pub path: PathBuf,
    pub file_type: RagProcessableFileType,
    pub internal_id: String,
    pub original_name: String,
}

#[derive(Debug)]
pub struct LoadedFile {
    pub file_type: RagProcessableFileType,
    pub content: String,
    pub syntetic_file_description: Option<String>,
    pub internal_id: String,
}

fn load_file(file: &RagProcessableFile) -> Result<LoadedFile> {
    let extracted_text = pdf_extract::extract_text(&file.path)
        .map_err(|err| anyhow!("Failed to extract text from PDF: {}", err))?;

    if extracted_text.trim().is_empty() {
        println!(
            "Warning: No text could be extracted from '{}'. It may be an image-only PDF.",
            file.path.display()
        );
    }

    Ok(LoadedFile {
        file_type: RagProcessableFileType::Pdf,
        content: extracted_text,
        internal_id: file.internal_id.clone(),
        syntetic_file_description: None,
    })
}