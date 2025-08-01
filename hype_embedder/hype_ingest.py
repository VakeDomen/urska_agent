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
from langchain_text_splitters import MarkdownHeaderTextSplitter, RecursiveCharacterTextSplitter, TextSplitter
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
        """"
        You will be given a chunk of text relating in some way to UP FAMNIT (University of Primorska - \
        Faculty of Mathematics, Natural Sciences, and Information Technologies). 


        Analyze the input text and generate all questions a student could ask that can be answered by the contents of the text. 
        It's important that the questions be exhaustive and understandable without context. 
        Named entities should always be referenced by their full name or short versions (like FAMNIT), but always referenced. 
        Only answer with questions, where each question should be written on its own line (separated by newline) with prefix: -. 
        It is especially important to generate only questions (many questions) when the text contains a table.
        If a question regards a person, a study program, or a particular year, always state the full name/information 
        in the question, especially regarding study programs. Start with most obvious simple questions and slowly ramp up in complexity.
        Make sure to exhaust all questions.



        -------------- Example Output to follow after </think>: --------------
        Who is Jernej Vičič?
        What is Eduroam and who can use it?
        What year is the class Applied Statistics offered?
        Who teaches Programming III (3) course?
        Who teaches Analysis III (3) – Functions of Many Variables course?
        How many internally selected elective courses do I have to choose in second year of Computer Science Bachlors?
        What courses are offered in the second year of computer science bachelor's?
        How many elective courses can I select in the third year of bachelor's?
        What is the typical class size for undergraduate courses in Computer Science?
        How is the academic year structured (semesters, exams, etc.)?
        Are there any student organizations or clubs related to Computer Science?
        What kind of career support does UP FAMNIT offer to its students?
        Is it possible to take an internship abroad during the Bachelor's program?
        Where can I find a table of courses offered in the masters program of Bioinformatics?
        Where can I find the official link to the master's thesis guidelines for Mathematical Sciences and Computer Science at UP FAMNIT?
        Where can I find the procedure for submitting a master’s thesis at UP FAMNIT?
        What are the requirements for obtaining a Bachelor’s degree in Computer Science at UP FAMNIT?
        What are the accommodation options for students at UP FAMNIT?
        What are the tuition fees for international students?
        Is there any financial aid or scholarship opportunities available?
        Is there a language requirement for international students?
        What is the grading system used at UP FAMNIT?
        How can I apply for a Bachelor’s program at UP FAMNIT?
        What are the deadlines for applying to the Bachelor’s programs?
        Is there an entrance exam for the Bachelor’s programs?
        
        """
    )

    user_template = ("""
        -------------- Text to ask about: START --------------
        {chunk}
        -------------- Text to ask about: END --------------

        -------------- Additional notes: --------------
        * Always state the full name/information (e.g. 2nd year bachlors Computer Science)
        * Always adress the information itself and not the document
        * Often will information be found in the document link (education/master/computer/science -> Computer Science Masters programm)
        * If applicable always accompany the program with the year
        * Only anser with a sequence of questions and no additional text. First Question should start with "What..."
        * Speak in first persion (e.g. How many elective courses must I select in second year of Computer Science Bachlors)
        * Only ask about the text in the "Text to ask about" block
        * Exhaust all possible questions (the more the better)


        Generate exhaustive questions now.
        """
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
    # splitter = RecursiveCharacterTextSplitter(
    #     chunk_size=5000,
    #     chunk_overlap=300,
    #     separators = [
    #         ["\n## ", TextSplitter.position.START],
    #         ["\n### ", TextSplitter.position.START],
    #         ["\n```", TextSplitter.position.BOTH],
    #         "\n\n",
    #         [".", TextSplitter.position.END],
    #         "\n",
    #         " ",
    #         ""
    #     ]
    # )

    markdown_headers = [
        ("#", "Header 1"),
        ("##", "Header 2"),
    ]

    # Initialize the Markdown Header Text Splitter
    splitter = MarkdownHeaderTextSplitter(
        headers_to_split_on=markdown_headers
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
        print(f"Took: {seconds} seconds to insert {len(questions)} questions to Qdrant | Chunk: {idx+1}/{len(chunks)}")
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
