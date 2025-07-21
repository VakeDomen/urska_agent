use std::{fs, io, path::Path, sync::{atomic::{AtomicUsize, Ordering}, Arc}};

use rayon::{iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator}, ThreadPoolBuilder};
use tokio::runtime::Handle;
use crate::rag::{comm::qdrant::insert_chunks_to_qdrant, models::RagProcessableFileType, processing::ChunkingStrategy, Rag, RagProcessableFile};

mod rag;


#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();
    let rag = Arc::new(Rag::default());

    // collect files up front
    let files = match find_processable_files("./resources/") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error scanning directory: {}", e);
            return;
        }
    };

    let total = files.len();
    let handle = Handle::current();

    let done = Arc::new(AtomicUsize::new(0));

    let pool = ThreadPoolBuilder::new()
        .num_threads(5)
        .build()
        .expect("couldn’t build rayon pool");

    pool.install(|| {
        files
            .into_par_iter()
            .for_each(|file| {
                // run the async insert on the tokio runtime
                let rag = Arc::clone(&rag);
                let result = handle.block_on(async {
                    rag.insert_with_strategy(file.clone(), ChunkingStrategy::Word(512, 128)).await
                });

                // bump our counter now that one has finished
                let idx = done.fetch_add(1, Ordering::SeqCst) + 1;
                println!("({}/{}) Inserting {:#?}", idx, total, file);

                match result {
                    Ok(_) => println!("-> File inserted"),
                    Err(err) => println!("Something went wrong: {:#?}", err.to_string()),
                }
            });
    });
}


pub fn find_processable_files(root_path_str: &str) -> io::Result<Vec<RagProcessableFile>> {
    let root_path = Path::new(root_path_str);
    let mut processable_files = Vec::new();
    visit_dirs(root_path, root_path, &mut processable_files)?;
    Ok(processable_files)
}

fn visit_dirs(
    dir: &Path,
    base_path: &Path,
    files: &mut Vec<RagProcessableFile>,
) -> io::Result<()> {
    // Check if the current path is a directory
    if dir.is_dir() {
        // Iterate over the entries in the directory
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            // If the entry is a directory, recurse into it
            if path.is_dir() {
                visit_dirs(&path, base_path, files)?;
            } 
            // If the entry is a file, process it
            else if path.is_file() {
                // Determine the file type from the extension
                let file_type = path.extension().and_then(|s| s.to_str()).and_then(|ext| {
                    match ext.to_lowercase().as_str() {
                        "txt" => Some(RagProcessableFileType::Text),
                        "md" => Some(RagProcessableFileType::Markdown),
                        "pdf" => Some(RagProcessableFileType::Pdf),
                        _ => None,
                    }
                });

                // If the file type is one we can process
                if let Some(ft) = file_type {
                    let original_name = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();

                    // Use the full path as a unique internal ID
                    let internal_id = uuid::Uuid::new_v4().to_string();

                    // Determine tags based on the parent subfolder name
                    let mut tags = None;
                    if let Some(parent_dir) = path.parent() {
                        // Only add a tag if the file is in a subfolder of the base path
                        if parent_dir != base_path {
                            if let Some(subfolder_name) = parent_dir.file_name().and_then(|n| n.to_str()) {
                                tags = Some(vec![subfolder_name.to_string()]);
                            }
                        }
                    }

                    // Create the struct with all the gathered information
                    let rag_file = RagProcessableFile {
                        path: path.clone(),
                        file_type: ft,
                        internal_id,
                        original_name,
                        file_description: Some("".to_string()), // Per request
                        tags,
                    };
                    files.push(rag_file);
                }
            }
        }
    }
    Ok(())
}