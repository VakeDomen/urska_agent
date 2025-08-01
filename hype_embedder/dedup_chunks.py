import json
from collections import defaultdict

# File paths
INPUT_FILE = 'chunks.jsonl'
OUTPUT_FILE = 'deduped_chunks.jsonl'

def deduplicate_chunks(input_file, output_file):
    # Dictionary to store chunks by their document_name and seq_num
    chunks_dict = defaultdict(list)

    # Read the input file and populate the dictionary
    with open(input_file, 'r') as f:
        for line in f:
            chunk = json.loads(line)
            key = (chunk['document_name'], chunk['seq_num'])
            if key not in chunks_dict:
                chunks_dict[key] = chunk

    # Write the deduplicated chunks to the output file
    with open(output_file, 'w') as f:
        for chunk in chunks_dict.values():
            json.dump(chunk, f, ensure_ascii=False)
            f.write('\n')

if __name__ == "__main__":
    deduplicate_chunks(INPUT_FILE, OUTPUT_FILE)
