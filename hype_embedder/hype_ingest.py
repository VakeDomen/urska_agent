"""
ingest.py

End‑to‑end pipeline that

1.   walks through every file under RESOURCE_FOLDER,
2.   produces a short summary of each file,
3.   splits the file into Markdown‑aware chunks,
4.   asks an Ollama LLM to generate HyPE questions for each chunk,
5.   embeds all questions from the chunk in one call,
6.   writes one Qdrant point per question, with uuid, vector and rich payload,
7.   runs several files in parallel (thread pool),
8.   keeps a simple progress log so it can resume after interruption.

❗ Environment variables are taken from .env (see sample provided by the user).
"""
from datetime import datetime
import os
import json
import uuid
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor, as_completed

from dotenv import load_dotenv
from tqdm import tqdm

from langchain_ollama import ChatOllama, OllamaEmbeddings
from langchain_text_splitters import RecursiveCharacterTextSplitter
from langchain.prompts.chat import (
    ChatPromptTemplate,
    HumanMessagePromptTemplate,
    SystemMessagePromptTemplate,
)
import re
from qdrant_client import QdrantClient
from qdrant_client.http import models as qm

# ==== configuration ====
load_dotenv()

RESOURCE_FOLDER = Path(os.getenv("RESOURCE_FOLDER", "./resources/english"))
MAX_WORKERS = int(os.getenv("MAX_WORKERS", "10"))
PROGRESS_FILE = Path("progress.json")

OLLAMA_BASE = f'{os.getenv("OLLAMA_HOST")}:{os.getenv("OLLAMA_PORT")}'
LLM_MODEL = os.getenv("LLM_MODEL")
EMBEDDING_MODEL = os.getenv("EMBEDDING_MODEL")

QDRANT_SERVER = os.getenv("QDRANT_SERVER")
QDRANT_COLLECTION = os.getenv("QDRANT_COLLECTION")
EMBEDDING_DIMENSION = int(os.getenv("EMBEDDING_DIMENSION", "1024"))
THINK_BLOCKS = re.compile(r"<think>.*?</think>", re.IGNORECASE | re.DOTALL)

# ==== models and clients ====
llm = ChatOllama(
    base_url=OLLAMA_BASE,
    model=LLM_MODEL,
    temperature=0.0,
)

embedder = OllamaEmbeddings(
    base_url=OLLAMA_BASE,
    model=EMBEDDING_MODEL,
)

qclient = QdrantClient(
    host="127.0.0.1",
    grpc_port=6334,
    prefer_grpc=True,
)

if QDRANT_COLLECTION not in [c.name for c in qclient.get_collections().collections]:
    qclient.create_collection(
        collection_name=QDRANT_COLLECTION,
        vectors_config=qm.VectorParams(
            size=EMBEDDING_DIMENSION,
            distance=qm.Distance.COSINE,
        ),
    )

# ==== utility helpers ====
def load_progress() -> set[str]:
    if PROGRESS_FILE.exists():
        with PROGRESS_FILE.open() as f:
            return set(json.load(f))
    return set()


def save_progress(done: set[str]) -> None:
    PROGRESS_FILE.write_text(json.dumps(sorted(done), indent=2))


def short_summary(text: str) -> str:
    """Ask the model for a concise summary (≈ one paragraph)."""
    prompt = ChatPromptTemplate.from_messages(
        [
            SystemMessagePromptTemplate.from_template(
                "Summarize the following document in one concise paragraph."
            ),
            HumanMessagePromptTemplate.from_template("{document}"),
        ]
    )
    message = prompt.format_prompt(document=text).to_messages()
    return llm.invoke(message).content.strip()


def hype_questions(chunk: str) -> list[str]:
    """Generate HyPE questions that start with '-'."""
    system_template = (
        "You will be given a passage from a document.\n"
        "Your task is to analyze the context text (passage) and "
        "generate essential questions that, when answered, capture the main points "
        "and core meaning of the text.\n"
        "The questions should be exhaustive and understandable without context. "
        "Named entities should be referenced by their full name when possible.\n"
        "However add questions that are diverse in topic.\n"
        "Only answer with questions and each question should be on its own line with prefix: -\n"
        "The answer to each question must be found in the passage.\n\n"
    )

    user_template = (
        "You will be given a chunk of text relating in some way to UP FAMNIT (University of Primorska – "
        "Faculty of Mathematics, Natural Sciences, and Information Technologies).\n\n"
        "-------------- Text to ask about: START --------------\n"
        "{chunk}\n"
        "-------------- Text to ask about: END --------------\n\n"
        "Generate exhaustive questions now."
    )

    prompt = ChatPromptTemplate.from_messages(
        [
            SystemMessagePromptTemplate.from_template(system_template),
            HumanMessagePromptTemplate.from_template(user_template),
        ]
    )


    message = prompt.format_prompt(chunk=chunk).to_messages()
    now = datetime.now()
    raw = llm.invoke(message).content
    cleaned = THINK_BLOCKS.sub("", raw).strip()
    questions = [q.lstrip("- ").strip() for q in cleaned.splitlines() if q.strip()]
    then = datetime.now()    
    tdelta = now - then
    seconds = tdelta.total_seconds()
    print(f"Took: {seconds} seconds to generate {len(questions)} questions")

    return questions


def embed_questions(questions: list[str]) -> list[list[float]]:
    """Embed questions in one batch call."""
    return embedder.embed_documents(questions)


def insert_points(
    vectors: list[list[float]],
    questions: list[str],
    chunk_text: str,
    file_name: str,
    chunk_id: int,
) -> None:
    payloads = []
    ids = []
    for vec, q in zip(vectors, questions):
        ids.append(str(uuid.uuid4()))
        payloads.append(
            {
                "question": q,
                "chunk": chunk_text,
                "file_name": file_name,
                "chunk_index": chunk_id,
            }
        )
    qclient.upsert(
        collection_name=QDRANT_COLLECTION,
        points=qm.Batch(
            ids=ids,
            vectors=vectors,
            payloads=payloads,
        ),
    )


def process_file(path: Path) -> str:
    text = path.read_text(encoding="utf‑8", errors="ignore")
    # summary = short_summary(text)

    now = datetime.now()    
    splitter = RecursiveCharacterTextSplitter(
        chunk_size=5000,
        chunk_overlap=300,
        separators=["\n\n", "\n", " "]
    )
    chunks = splitter.split_text(text)
    then = datetime.now()    
    tdelta = now - then
    seconds = tdelta.total_seconds()
    print(f"Took: {seconds} seconds to plit file into {len(chunks)} chunks")

    for idx, chunk in enumerate(chunks):
        questions = hype_questions(chunk)
        if not questions:
            continue
        now = datetime.now()
        vectors = embed_questions(questions)
        then = datetime.now()    
        tdelta = now - then
        seconds = tdelta.total_seconds()
        print(f"Took: {seconds} seconds to embedd {len(questions)} questions")

        now = datetime.now()
        insert_points(vectors, questions, chunk, str(path), idx)
        then = datetime.now()    
        tdelta = now - then
        seconds = tdelta.total_seconds()
        print(f"Took: {seconds} seconds to insert {len(questions)} questions to Qdrant")
    return str(path)


# ==== main driver ====
def main() -> None:
    done = load_progress()
    files = sorted(
        p for p in RESOURCE_FOLDER.rglob("*") if p.is_file() and str(p) not in done
    )

    if not files:
        print("Nothing to do.")
        return

    with ThreadPoolExecutor(max_workers=MAX_WORKERS) as pool:
        futures = {pool.submit(process_file, p): p for p in files}
        for fut in tqdm(as_completed(futures), total=len(futures)):
            finished_file = fut.result()
            done.add(finished_file)
            save_progress(done)

    print("Ingestion finished.")


if __name__ == "__main__":
    main()
