import json
import os
import subprocess
import tempfile
from datetime import datetime
from agent import run_agent

TESTSET_PATH = "testset.jsonl"
RAGCHECKER_OUTPUT = "ragchecker_results.json"
EVALUATION_LOG = "failed_cases.log"

def load_testset():
    with open(TESTSET_PATH, "r") as f:
        return [json.loads(line) for line in f]

def generate_agent_outputs(testset):
    print(f"Running agent on {len(testset)} questions...")
    results = []
    for entry in testset:
        question = entry["question"]
        expected = entry.get("expected_answer", "")
        answer = run_agent(question)
        results.append({
            "question": question,
            "expected_answer": expected,
            "answer": answer,
            "context": ""  # optional, can add retrieval context here
        })
    return results

def save_generated_data(examples, path):
    with open(path, "w") as f:
        for ex in examples:
            f.write(json.dumps(ex) + "\n")

def run_ragchecker(input_path, output_path):
    print("Running RAGChecker...")
    cmd = [
        "ragchecker", "evaluate",
        "--input", input_path,
        "--output", output_path
    ]
    subprocess.run(cmd, check=True)

def analyze_failures(output_path, log_path):
    with open(output_path, "r") as f:
        data = json.load(f)

    failed = [ex for ex in data if ex.get("factuality", 1.0) < 0.5]

    print(f"\nFound {len(failed)} failing examples.")
    if failed:
        with open(log_path, "w") as log:
            log.write(f"RAGChecker failures - {datetime.now()}\n\n")
            for ex in failed:
                log.write(f"❌ Question: {ex['question']}\n")
                log.write(f"   Answer: {ex['answer']}\n")
                log.write(f"   Expected: {ex.get('expected_answer', '')}\n")
                log.write(f"   Factuality: {ex.get('factuality', 'N/A')}\n\n")
        print(f"Failures saved to: {log_path}")

def main():
    testset = load_testset()
    results = generate_agent_outputs(testset)

    with tempfile.NamedTemporaryFile(delete=False, suffix=".jsonl") as tmp_input:
        input_path = tmp_input.name
        save_generated_data(results, input_path)

    run_ragchecker(input_path, RAGCHECKER_OUTPUT)
    analyze_failures(RAGCHECKER_OUTPUT, EVALUATION_LOG)

if __name__ == "__main__":
    main()
