use anyhow::Result;
use loaded_data::LoadedFile;
use markdown::MarkdownFileLoader;
use pdf::PdfFileLoader;
use text::TextFileLoader;

use super::{models::RagProcessableFileType, RagProcessableFile};

pub mod loaded_data;
mod markdown;
mod pdf;
mod text;

trait FileLoader {
    fn load_file(file: &RagProcessableFile) -> Result<LoadedFile>;
}

pub fn load_file(file: &RagProcessableFile) -> Result<LoadedFile> {
    match file.file_type {
        RagProcessableFileType::Text => TextFileLoader::load_file(file),
        RagProcessableFileType::Markdown => MarkdownFileLoader::load_file(file),
        RagProcessableFileType::Pdf => PdfFileLoader::load_file(file),
    }
}
