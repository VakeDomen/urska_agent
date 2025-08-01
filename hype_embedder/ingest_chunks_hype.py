"""
hypify.py
Script that reads a JSONL file containing document chunks and generates HyPE questions for each chunk.
1. Reads a JSONL file named "deduped_chunks.jsonl"
2. For each chunk, generates HyPE questions (starting with "-")
3. Deduplicates the generated questions to avoid repetition
4. Embeds the unique questions
5. Writes one Qdrant point per question, with uuid, vector and rich payload
6. Keeps a simple progress log so it can resume after interruption.
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
DEDUPED_CHUNKS_FILE = Path("deduped_chunks.jsonl")
MAX_WORKERS = int(os.getenv("MAX_WORKERS", "10"))
PROGRESS_FILE = Path("progress.json")
OLLAMA_BASE = f'{os.getenv("OLLAMA_HOST")}:{os.getenv("OLLAMA_PORT")}'
LLM_MODEL = os.getenv("LLM_MODEL")
EMBEDDING_MODEL = os.getenv("EMBEDDING_MODEL")
QDRANT_SERVER = os.getenv("QDRANT_SERVER")
QDRANT_COLLECTION = os.getenv("QDRANT_COLLECTION")
EMBEDDING_DIMENSION = int(os.getenv("EMBEDDING_DIMENSION", "1024"))
REVIEW_FILE = Path("questions_review.jsonl") 


# ==== models and clients ====
llm = ChatOllama(
    base_url=OLLAMA_BASE,
    model=LLM_MODEL,
    temperature=0.7,          # Sampling: Higher diversity, lower predictability
    top_p=0.8,               # Sampling: Nucleus sampling, focuses on high-probability tokens
    top_k=20,                # Sampling: Limits choices to top-k most likely tokens
    min_p=0.0,               # Sampling: Prevents very low-probability tokens from being chosen
    presence_penalty=1.5,    # Penalizes repeated tokens, reducing loops and repetitions
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

def save_for_review(chunk_data: dict, questions: list[str]) -> None:
    """NEW: Save chunk data and generated questions to review file"""
    review_entry = {
        "document_id": chunk_data["document_id"],
        "document_name": chunk_data["document_name"],
        "seq_num": chunk_data["seq_num"],
        "chunk": chunk_data["chunk"],
        "generated_questions": questions
    }
    
    with REVIEW_FILE.open("a", encoding="utf-8") as f:
        f.write(json.dumps(review_entry, ensure_ascii=False) + "\n")


def hype_questions(chunk: str) -> list[str]:
    """Generate HyPE questions that start with "-". This is the same as in the provided script."""
    # system_template = (
    #     """"
    #     You will be given a chunk of text relating in some way to UP FAMNIT (University of Primorska - \
    #     Faculty of Mathematics, Natural Sciences, and Information Technologies). 
    #     Analyze the input text and generate all questions a student could ask that can be answered by the contents of the text. 
    #     It's important that the questions be exhaustive and understandable without context. 
    #     Named entities should always be referenced by their full name or short versions (like FAMNIT), but always referenced. 
    #     Only answer with questions, where each question should be written on its own line (separated by newline) with prefix: -. 
    #     It is especially important to generate only questions (many questions) when the text contains a table.
    #     If a question regards a person, a study program, or a particular year, always state the full name/information 
    #     in the question, especially regarding study programs. Start with most obvious simple questions and slowly ramp up in complexity.
    #     Make sure to exhaust all questions.
    #     -------------- Example Output to follow after  </think>: --------------
    #     Who is Jernej Vičič?
    #     What is Eduroam and who can use it?
    #     What year is the class Applied Statistics offered?
    #     Who teaches Programming III (3) course?
    #     Who teaches Analysis III (3) – Functions of Many Variables course?
    #     How many internally selected elective courses do I have to choose in second year of Computer Science Bachlors?
    #     What courses are offered in the second year of computer science bachelor's?
    #     How many elective courses can I select in the third year of bachelor's?
    #     What is the typical class size for undergraduate courses in Computer Science?
    #     How is the academic year structured (semesters, exams, etc.)?
    #     Are there any student organizations or clubs related to Computer Science?
    #     What kind of career support does UP FAMNIT offer to its students?
    #     Is it possible to take an internship abroad during the Bachelor's program?
    #     Where can I find a table of courses offered in the masters program of Bioinformatics?
    #     Where can I find the official link to the master's thesis guidelines for Mathematical Sciences and Computer Science at UP FAMNIT?
    #     Where can I find the procedure for submitting a master’s thesis at UP FAMNIT?
    #     What are the requirements for obtaining a Bachelor’s degree in Computer Science at UP FAMNIT?
    #     What are the accommodation options for students at UP FAMNIT?
    #     What are the tuition fees for international students?
    #     Is there any financial aid or scholarship opportunities available?
    #     Is there a language requirement for international students?
    #     What is the grading system used at UP FAMNIT?
    #     How can I apply for a Bachelor’s program at UP FAMNIT?
    #     What are the deadlines for applying to the Bachelor’s programs?
    #     Is there an entrance exam for the Bachelor’s programs?
    #     """
    # )

    system_template = """
    You are a question generation specialist for UP FAMNIT (University of Primorska - Faculty of Mathematics, Natural Sciences, and Information Technologies) student resources.
    
    TASK: Analyze the input text and generate ONLY practical, student-focused questions that can be directly answered by the contents of the text.
    
    CRITICAL RULES:
    1. ONLY generate questions where the complete answer exists in the provided text
    2. NEVER generate philosophical, speculative, or "flavor" questions (e.g., "What is the relationship between...", "What skills are essential for...")
    3. ALWAYS use first-person perspective as if a student is asking ("How many courses must I take...")
    4. ALWAYS reference named entities by full name or appropriate short form (e.g., "UP FAMNIT", "Computer Science Bachelor's program")
    5. FOR study programs, ALWAYS specify the exact program name and level (Bachelors/Masters/PhD) AND year when relevant
    6. FOR tables, generate questions about specific data points but avoid redundant variations
    7. NEVER ask about the document itself (e.g., "What is the name of the study program described in the text?")
    
    QUALITY OVER QUANTITY:
    - Focus on generating 15-50 HIGH-QUALITY questions (25 max for small chunks, 70 max for large chunks)
    - Prioritize questions students would ACTUALLY ask when navigating university resources
    - If the text describes multiple courses taught by a professor, generate ONE question per course (in this case it's allowed to pass the maximum quantitiy)
    - If the text contains program requirements, generate questions about specific requirements, not general concepts
    
    BAD EXAMPLES (NEVER GENERATE THESE):
    - "What is the relationship between the natural sciences and psychology in the Biopsychology study programme?"
    - "What skills are essential for conducting research in biopsychology?"
    - "What is the name of the study programme described in the text?"
    - "Why is this program important for students?"
    
    GOOD EXAMPLES (GENERATE QUESTIONS LIKE THESE):
    - How many elective courses must I select in the second year of the Computer Science Bachelor's program?
    - Who teaches Programming III (3) course in the 2023/2024 academic year?
    - What are the tuition fees for international students in the Mathematics Master's program?
    - Where can I find the application deadline for the Bioinformatics Master's program?
    - What is the minimum grade required to pass Analysis I course?
    
    OUTPUT FORMAT:
    - ONLY output questions, nothing else
    - Each question must be on its own line
    - Prefix each question with "- "
    - Start with the most basic factual questions, then progress to more specific ones
    - Ensure every question is understandable without additional context
    """
    user_template = ("""
    -------------- Text to ask about: START --------------
    {chunk}
    -------------- Text to ask about: END --------------
    
    -------------- Processing Instructions --------------
    1. Scan the text for concrete facts, requirements, dates, people, and program details
    2. Generate ONLY questions where the answer is explicitly stated in the text
    3. For program information, always include: 
       - Program level (Bachelors/Masters/PhD)
       - Full program name
       - Specific year if mentioned
       - Specific academic year if relevant
    4. For courses, generate questions about:
       - Course instructors
       - Credit values
       - Prerequisites
       - When offered (semester/year)
       - Requirements to pass
    5. For people, generate questions about:
       - Their role/position
       - Courses they teach
       - Contact information
       - Office hours
    6. For tables, generate questions about specific data points, NOT the table structure
    7. STOP generating when you reach 20 questions for small chunks or 70 for large chunks
    8. NEVER generate more than one question for the same fact
    
    REMEMBER: 
        - Only generate questions a student would actually ask when trying to navigate university resources.
        - We are cuttently in academic year 2024-25
    
    Generate relevant questions now.
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
    cleaned = llm.invoke(message).content.strip()
    # cleaned = THINK_BLOCKS.sub("", raw).strip()
    # Split lines, remove leading/trailing dashes and spaces, and filter out empty strings
    raw_questions = [q.strip().lstrip("- ").strip() for q in cleaned.splitlines() if q.strip()]
    
    # ===== DEDUPLICATION STEP =====
    # Convert to a set to remove duplicates, then back to a list to maintain order (as much as possible)
    # Note: Sets don't guarantee order, but since we're processing sequentially, the order is mostly preserved.
    unique_questions = list(set(raw_questions))
    
    then = datetime.now()    
    tdelta = now - then
    seconds = tdelta.total_seconds()
    print(f"Took: {seconds} seconds to generate {len(unique_questions)} unique questions")
    return unique_questions

def embed_questions(questions: list[str]) -> list[list[float]]:
    """Embed questions in one batch call."""
    return embedder.embed_documents(questions)

def insert_points(
    vectors: list[list[float]],
    questions: list[str],
    chunk_text: str,
    document_id: str,
    document_name: str,
    seq_num: int,
) -> None:
    payloads = []
    ids = []
    for vec, q in zip(vectors, questions):
        ids.append(str(uuid.uuid4()))
        payloads.append(
            {
                "question": q,
                "chunk": chunk_text,
                "document_id": document_id,
                "document_name": document_name,
                "seq_num": seq_num,
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

def process_chunk(chunk_data: dict) -> str:
    """Process a single chunk from the JSONL file."""
    chunk_text = chunk_data["chunk"]
    document_id = chunk_data["document_id"]
    document_name = chunk_data["document_name"]
    seq_num = chunk_data["seq_num"]

    # Generate questions
    questions = hype_questions(chunk_text)
    if not questions:
        return f"Skipped (no questions generated) - {document_name}:{seq_num}"

    # save_for_review(chunk_data, questions)

    # Embed questions
    now = datetime.now()
    vectors = embed_questions(questions)
    then = datetime.now()    
    tdelta = now - then
    seconds = tdelta.total_seconds()
    print(f"Took: {seconds} seconds to embed {len(questions)} unique questions")

    # Insert into Qdrant
    now = datetime.now()
    insert_points(vectors, questions, chunk_text, document_id, document_name, seq_num)
    then = datetime.now()    
    tdelta = now - then
    seconds = tdelta.total_seconds()
    print(f"Took: {seconds} seconds to insert {len(questions)} unique questions to Qdrant | Document: {document_name} | Seq: {seq_num}")

    return f"Processed - {document_name}:{seq_num}"


def main() -> None:
    # Load progress
    done = load_progress()

    # Read all chunks from the JSONL file
    all_chunks = []
    try:
        with DEDUPED_CHUNKS_FILE.open("r", encoding="utf-8") as f:
            for line_num, line in enumerate(f, start=1):
                try:
                    chunk_data = json.loads(line.strip())
                    all_chunks.append(chunk_data)
                except json.JSONDecodeError as e:
                    print(f"ERROR: Failed to parse JSON on line {line_num}. Skipping. Error: {e}")
                    continue
    except FileNotFoundError:
        print(f"ERROR: JSONL file '{DEDUPED_CHUNKS_FILE}' not found.")
        return
    except IOError as e:
        print(f"ERROR: Failed to read file '{DEDUPED_CHUNKS_FILE}'. Error: {e}")
        return

    # Filter out chunks that have already been processed
    chunks_to_process = [
        chunk for chunk in all_chunks 
        if f"{chunk['document_name']}:{chunk['seq_num']}" not in done
    ]

    if not chunks_to_process:
        print("Nothing to do. All chunks have been processed.")
        return

    print(f"Found {len(chunks_to_process)} chunks to process.")

    # Process chunks in parallel
    with ThreadPoolExecutor(max_workers=MAX_WORKERS) as pool:
        futures = {pool.submit(process_chunk, chunk): chunk for chunk in chunks_to_process}
        for fut in tqdm(as_completed(futures), total=len(futures)):
            try:
                result = fut.result()
                # Extract the identifier for the chunk
                identifier = result.split(" - ")[1]
                done.add(identifier)

                # === CRITICAL: Save progress inside a try-except ===
                try:
                    save_progress(done)
                except Exception as e:
                    print(f"WARNING: Failed to save progress file. Progress might be lost. Error: {e}")
                    # Optionally, log the error to a file for debugging
                    # with open("error.log", "a") as err_log:
                    #     err_log.write(f"Failed to save progress: {e}\n")
                    # Don't re-raise, just continue
            except Exception as e:
                print(f"ERROR: Processing failed for a chunk. Error: {e}")
                continue

    print("Processing completed.")

if __name__ == "__main__":
    main()