import os
import json

# Step 1: Recursively find all file names in the specified directory
resources_dir = './resources/english/'
all_files = []

for root, dirs, files in os.walk(resources_dir):
    for file in files:
        full_path = os.path.join(root, file)
        all_files.append(full_path)

# Step 2: Read the chunk.jsonl file and extract document_name fields
chunk_file_path = './chunks.jsonl'
processed_docs = set()

with open(chunk_file_path, 'r') as chunk_file:
    for line in chunk_file:
        data = json.loads(line)
        if 'document_name' in data:
            processed_docs.add(data['document_name'])

# Step 3: Separate the files into processed and not processed
progress_pre = []
progress_todo = []

for file_path in all_files:
    file_name = os.path.basename(file_path)
    if file_name in processed_docs:
        progress_pre.append(file_path)
    else:
        progress_todo.append(file_path)

# Step 4: Write the results into the respective JSON files
with open('progress_pre.json', 'w') as pre_file:
    json.dump(progress_pre, pre_file, indent=2)

with open('progress_todo.json', 'w') as todo_file:
    json.dump(progress_todo, todo_file, indent=2)

print("Processing complete. Results written to progress_pre.json and progress_todo.json.")
