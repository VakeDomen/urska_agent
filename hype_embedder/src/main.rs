use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Write},
    path::Path,
    sync::{atomic::{AtomicUsize, Ordering}, Arc, Mutex},
};

use rayon::prelude::*;
use tokio::runtime::Handle;

use crate::rag::{comm::qdrant::insert_chunks_to_qdrant, models::RagProcessableFileType, processing::ChunkingStrategy, Rag, RagProcessableFile};

mod rag;

const CHECKPOINT_FILE: &str = "./processed.json";

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();
    let rag = Arc::new(Rag::default());
    let handle = Handle::current();

    let processed: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(load_processed_files(CHECKPOINT_FILE)));

    let files = match find_processable_files("./resources/") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error scanning directory: {}", e);
            return;
        }
    };

    let files: Vec<_> = files
        .into_iter()
        .filter(|file| {
            let processed = processed.lock().unwrap();
            !processed.contains(&file.internal_id)
        })
        .collect();

    let total = files.len();
    let done = Arc::new(AtomicUsize::new(0));

    rayon::ThreadPoolBuilder::new()
        .num_threads(10)
        .build()
        .expect("couldn’t build rayon pool")
        .install(|| {
            files.into_par_iter().for_each(|file| {
                let rag = Arc::clone(&rag);
                let processed = Arc::clone(&processed);
                let original_name = file.original_name.clone();

                let result = handle.block_on(async {
                    rag.insert_with_strategy(file.clone(), ChunkingStrategy::Word(512, 128)).await
                });

                let idx = done.fetch_add(1, Ordering::SeqCst) + 1;
                println!("({}/{}) Processing {}", idx, total, file.original_name);

                match result {
                    Ok(_) => {
                        println!("-> File inserted");
                        let mut set = processed.lock().unwrap();
                        set.insert(original_name);
                        save_processed_files(CHECKPOINT_FILE, &set);
                    }
                    Err(err) => println!("-> Error inserting file: {:#?}", err),
                }
            });
        });
}

fn load_processed_files(path: &str) -> HashSet<String> {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_processed_files(path: &str, set: &HashSet<String>) {
    if let Ok(json) = serde_json::to_string_pretty(set) {
        let _ = File::create(path).and_then(|mut f| f.write_all(json.as_bytes()));
    }
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
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                visit_dirs(&path, base_path, files)?;
            } else if path.is_file() {
                let file_type = path.extension().and_then(|s| s.to_str()).and_then(|ext| {
                    match ext.to_lowercase().as_str() {
                        "txt" => Some(RagProcessableFileType::Text),
                        "md" => Some(RagProcessableFileType::Markdown),
                        "pdf" => Some(RagProcessableFileType::Pdf),
                        _ => None,
                    }
                });

                if let Some(ft) = file_type {
                    let original_name = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();

                    let internal_id = uuid::Uuid::new_v4().to_string();

                    let mut tags = None;
                    if let Some(parent_dir) = path.parent() {
                        if parent_dir != base_path {
                            if let Some(subfolder_name) = parent_dir.file_name().and_then(|n| n.to_str()) {
                                tags = Some(vec![subfolder_name.to_string()]);
                            }
                        }
                    }

                    let rag_file = RagProcessableFile {
                        path: path.clone(),
                        file_type: ft,
                        internal_id,
                        original_name,
                        file_description: Some("".to_string()),
                        tags,
                    };
                    files.push(rag_file);
                }
            }
        }
    }
    Ok(())
}
