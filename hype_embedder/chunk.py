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
from threading import Lock

# Configuration
load_dotenv()
RESOURCE_FOLDER = Path(os.getenv("RESOURCE_FOLDER", "./resources/AKTI_UP_STUDENTI/clean"))
MAX_WORKERS = int(os.getenv("MAX_WORKERS", "8"))
OUTPUT_FILE = Path(os.getenv("OUTPUT_FILE", "chunks_rules.jsonl"))
PROGRESS_FILE = Path("progress_pre.json")
OLLAMA_BASE = f"{os.getenv('OLLAMA_HOST')}:{os.getenv('OLLAMA_PORT')}"
LLM_MODEL = os.getenv("LLM_MODEL", "phi3")

# Models
llm = ChatOllama(
    base_url=OLLAMA_BASE,
    model=LLM_MODEL,
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
    # return llm.invoke(
    #     SUMMARY_PROMPT.format_prompt(document=text).to_messages()
    # ).content.strip().split("</think>").last().strip()
    return "Official rules and acts of University of Primorska"


def split_into_hierarchical_chunks(doc_location: str, text: str, max_chars: int = 5000) -> list[str]:
    """
    Return chunks no larger than max_chars.
    Strategy:
      1) If the whole file fits, return it.
      2) Else split by H2 (##). Prefix each chunk with H1 and that H2.
      3) For any H2 chunk still too large, split by H3 (###) and prefix with H1, H2, H3.
    Note: If a subchunk is still too large after H3 splitting, we do a gentle paragraph split
    while keeping the same header prefix for each piece.
    """
    lines = text.splitlines()
    h1 = None

    # collect H1
    for ln in lines:
        if ln.startswith("# "):
            h1 = ln.strip()
            break

    h1 = f"Document location: [{doc_location}]({doc_location})\n\n" + h1

    def sectionize(lines, marker):
        """Split lines by a markdown header marker ('## ' or '### ')."""
        sections = []
        current_title = None
        current_body = []
        for ln in lines:
            if ln.startswith(marker):
                if current_title or current_body:
                    sections.append((current_title, current_body))
                current_title = ln.strip()
                current_body = []
            else:
                current_body.append(ln)
        if current_title or current_body:
            sections.append((current_title, current_body))
        return sections

    def make_prefixed_chunk(prefix_lines, body_lines):
        return "\n".join(filter(None, prefix_lines + body_lines)).strip()

    def pack_sections(sections, prefix):
        """Greedily merge sections until max_chars reached."""
        chunks = []
        buf_body = []
        buf_prefix = None
        current_len = 0

        for title, body in sections:
            # Build section text
            section_text = make_prefixed_chunk(prefix + ([title] if title else []), body)
            if not buf_body:
                buf_body = section_text.splitlines()
                buf_prefix = prefix
                current_len = len(section_text)
            else:
                if current_len + len(section_text) + 1 <= max_chars:
                    buf_body.extend(section_text.splitlines())
                    current_len += len(section_text) + 1
                else:
                    chunks.append("\n".join(buf_body).strip())
                    buf_body = section_text.splitlines()
                    buf_prefix = prefix
                    current_len = len(section_text)

        if buf_body:
            chunks.append("\n".join(buf_body).strip())
        return chunks

    # Step 0: If the whole file fits, return as one chunk
    if len(text) <= max_chars:
        return [text.strip()]

    # Step 1: Split by H2
    h2_sections = sectionize(lines, "## ")

    final_chunks = []
    for h2_title, h2_body in h2_sections:
        h2_prefix = [h1] if h1 else []
        if h2_title:
            h2_prefix.append(h2_title)
        h2_text = make_prefixed_chunk(h2_prefix, h2_body)

        if len(h2_text) <= max_chars:
            final_chunks.append(h2_text)
        else:
            # Step 2: Split this H2 section by H3
            h3_sections = sectionize(h2_body, "### ")
            if not h3_sections:
                # No H3 — just pack by paragraphs
                paras = [(None, [p]) for p in "\n".join(h2_body).split("\n\n") if p.strip()]
                final_chunks.extend(pack_sections(paras, h2_prefix))
            else:
                # Prefix each H3 with H1 + H2 + H3
                h3_chunks = []
                for h3_title, h3_body in h3_sections:
                    h3_prefix = h2_prefix + ([h3_title] if h3_title else [])
                    h3_text = make_prefixed_chunk(h3_prefix, h3_body)
                    if len(h3_text) <= max_chars:
                        h3_chunks.append((h3_title, h3_body))
                    else:
                        # Too big even for H3, fall back to paragraph packing
                        paras = [(None, [p]) for p in "\n".join(h3_body).split("\n\n") if p.strip()]
                        packed = pack_sections(paras, h3_prefix)
                        final_chunks.extend(packed)
                # Step 3: Merge H3 subsections under the same H2 where possible
                final_chunks.extend(pack_sections(h3_chunks, h2_prefix))

    return final_chunks


def _split_h3_under(
    h1: str | None,
    h2: str | None,
    body_lines: list[str],
    max_chars: int,
    join_with_prefix,
    paragraph_split,
) -> list[str]:
    h3_sections = []
    current_h3 = None
    current_body = []

    def flush_h3():
        if current_h3 is not None or current_body:
            h3_sections.append((current_h3, current_body.copy()))

    for ln in body_lines:
        if ln.startswith("### "):
            flush_h3()
            current_h3 = ln.strip()
            current_body = []
        else:
            current_body.append(ln)
    flush_h3()

    # If no H3 found, fall back to paragraph split under H2
    if not h3_sections:
        prefix = [p for p in [h1, h2] if p]
        candidate = join_with_prefix(prefix, body_lines)
        if len(candidate) <= max_chars:
            return [candidate]
        return paragraph_split(prefix, body_lines)

    out = []
    for h3, body in h3_sections:
        prefix = [p for p in [h1, h2, h3] if p]
        candidate = join_with_prefix(prefix, body)
        if len(candidate) <= max_chars:
            out.append(candidate)
        else:
            # still too large, paragraph split under same prefix
            out.extend(paragraph_split(prefix, body))
    return out


def parse_link_from_filename(filename: str) -> str:
    """Reconstruct the original URL from the scraped *filename*."""
    name = filename.removesuffix(".md")
    # tokens = name.split("_")
    # try:
    #     en_idx = tokens.index("en")
    # except ValueError:
    #     en_idx = len(tokens) - 1
    # domain_tokens = tokens[: en_idx + 1]
    # path_tokens = tokens[en_idx + 1 :]
    # domain = ".".join(domain_tokens[:-1]) + "/" + domain_tokens[-1]
    # url = f"https://{domain}"
    # if any(path_tokens):
    #     url += "/" + "/".join(filter(bool, path_tokens))
    return filename.removesuffix(".md").replace("_", "/")
    # return url

def extract_keywords(path: Path) -> list[str]:
    """Return keywords derived from *path*."""
    # rel = str(path.relative_to(RESOURCE_FOLDER))
    # rel = rel.replace("www_famnit_upr_si_en", "").replace(".md", "")
    # tokens = re.split(r"[\\/_]", rel)
    # return [t for t in tokens if t]
    return ["rules", "act", "upr.si"]

OUTPUT_LOCK = Lock()  # put near your other globals

def append_records(records: list[dict]) -> None:
    """Write *records* to OUTPUT_FILE in JSONL format, one thread at a time."""
    payload = "\n".join(json.dumps(rec, ensure_ascii=False) for rec in records) + "\n"
    with OUTPUT_LOCK:
        with OUTPUT_FILE.open("a", encoding="utf-8") as f:
            f.write(payload)
            f.flush()
            os.fsync(f.fileno())


def process_file(path: Path) -> str:
    """Process a single file and return its path as a string."""
    text = path.read_text(encoding="utf-8", errors="ignore")
    summary = short_summary(text)
    link = parse_link_from_filename(path.name)
    keywords = extract_keywords(path)
    records: list[dict] = []
    for seq, chunk in enumerate(split_into_hierarchical_chunks(link, text)):
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
