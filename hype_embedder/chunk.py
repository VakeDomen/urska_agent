from __future__ import annotations
import json
import os
import re
import uuid
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from dotenv import load_dotenv
from tqdm import tqdm
from langchain_ollama import ChatOllama
from langchain.prompts.chat import (
    ChatPromptTemplate,
    HumanMessagePromptTemplate,
    SystemMessagePromptTemplate,
)

# Configuration
load_dotenv()
RESOURCE_FOLDER = Path(os.getenv("RESOURCE_FOLDER", "./resources/english"))
MAX_WORKERS = int(os.getenv("MAX_WORKERS", "8"))
OUTPUT_FILE = Path(os.getenv("OUTPUT_FILE", "chunks.jsonl"))
PROGRESS_FILE = Path("progress_pre.json")
OLLAMA_BASE = f"{os.getenv('OLLAMA_HOST')}:{os.getenv('OLLAMA_PORT')}"
LLM_MODEL = os.getenv("LLM_MODEL", "phi3")

# Models
llm = ChatOllama(
    base_url=OLLAMA_BASE,
    model=LLM_MODEL,
    temperature=0.0,
    host="http://hivecore.famnit.upr.si:6666"
)

SUMMARY_PROMPT = ChatPromptTemplate.from_messages(
    [
        SystemMessagePromptTemplate.from_template(
            "Summarize the following document in one concise paragraph."
        ),
        HumanMessagePromptTemplate.from_template("{document}"),
    ]
)

# Helpers
def short_summary(text: str) -> str:
    """Return a one-paragraph summary of *text* using the configured LLM."""
    return llm.invoke(
        SUMMARY_PROMPT.format_prompt(document=text).to_messages()
    ).content.strip().split("</think>")[1].strip()

def split_by_h2_with_h1_prefix(text: str) -> list[str]:
    """Split *text* into chunks at each H2 (##) heading."""
    if len(text) < 5000:
        return [text]
    lines = text.splitlines()
    chunks: list[str] = []
    current_h1: str | None = None
    current_chunk: list[str] = []

    def flush() -> None:
        if current_chunk:
            chunks.append("\n".join(current_chunk).strip())

    for line in lines:
        if line.startswith("# "):
            current_h1 = line
            continue
        if line.startswith("## "):
            flush()
            current_chunk.clear()
            if current_h1:
                current_chunk.append(current_h1)
            current_chunk.append(line)
            continue
        if current_chunk:
            current_chunk.append(line)
    flush()
    return [c for c in chunks if c]

def parse_link_from_filename(filename: str) -> str:
    """Reconstruct the original URL from the scraped *filename*."""
    name = filename.removesuffix(".md")
    tokens = name.split("_")
    try:
        en_idx = tokens.index("en")
    except ValueError:
        en_idx = len(tokens) - 1
    domain_tokens = tokens[: en_idx + 1]
    path_tokens = tokens[en_idx + 1 :]
    domain = ".".join(domain_tokens[:-1]) + "/" + domain_tokens[-1]
    url = f"https://{domain}"
    if any(path_tokens):
        url += "/" + "/".join(filter(bool, path_tokens))
    return url

def extract_keywords(path: Path) -> list[str]:
    """Return keywords derived from *path*."""
    rel = str(path.relative_to(RESOURCE_FOLDER))
    rel = rel.replace("www_famnit_upr_si_en", "").replace(".md", "")
    tokens = re.split(r"[\\/_]", rel)
    return [t for t in tokens if t]

def append_records(records: list[dict]) -> None:
    """Write *records* to OUTPUT_FILE in JSONL format."""
    with OUTPUT_FILE.open("a", encoding="utf-8") as f:
        for rec in records:
            json.dump(rec, f, ensure_ascii=False)
            f.write("\n")

def process_file(path: Path) -> str:
    """Process a single file and return its path as a string."""
    text = path.read_text(encoding="utf-8", errors="ignore")
    summary = short_summary(text)
    link = parse_link_from_filename(path.name)
    keywords = extract_keywords(path)
    records: list[dict] = []
    for seq, chunk in enumerate(split_by_h2_with_h1_prefix(text)):
        records.append(
            {
                "document_id": str(uuid.uuid4()),
                "document_name": path.name,
                "link": link,
                "seq_num": seq,
                "chunk": chunk,
                "summary": summary,
                "keywords": keywords,
            }
        )
    append_records(records)
    return str(path)


RESOURCE_FOLDER = Path("./resources/english")
PROGRESS_FILE = Path("progress_pre.json")

def load_progress() -> set[str]:
    """Load the progress from the progress file."""
    if PROGRESS_FILE.exists():
        done = set(json.loads(PROGRESS_FILE.read_text()))
        # Extract only the filenames from the stored paths
        return {Path(p).name for p in done}
    return set()

def save_progress(done: set[str]) -> None:
    """Save the progress to the progress file."""
    # Convert absolute paths to filenames for storage
    filenames = {Path(p).name for p in done}
    PROGRESS_FILE.write_text(json.dumps(sorted(filenames), indent=2))

def main() -> None:
    # Load the progress
    done_filenames = load_progress()

    # Get all markdown files and extract filenames
    all_files = [str(p) for p in RESOURCE_FOLDER.rglob("*.md")]
    all_filenames = {Path(p).name for p in all_files}

    # Filter out files that have already been processed
    files_to_process = [p for p in all_files if Path(p).name not in done_filenames]

    # Debugging output
    print(f"Total files found: {len(all_files)}")
    print(f"Files already done: {len(done_filenames)}")
    print(f"Files to process: {len(files_to_process)}")

    # Print some examples for debugging
    print("Examples of files to process:")
    for file in files_to_process[:5]:
        print(file)

    if not files_to_process:
        print("All files already processed.")
        return

    # Assuming process_file and other necessary functions are defined
    with ThreadPoolExecutor(max_workers=MAX_WORKERS) as pool:
        futures = {pool.submit(process_file, Path(p)): p for p in files_to_process}
        for fut in tqdm(as_completed(futures), total=len(futures)):
            finished = fut.result()
            done_filenames.add(Path(finished).name)
            save_progress(done_filenames)

    print("✅ Pre-processing complete. Output →", OUTPUT_FILE)

if __name__ == "__main__":
    main()
