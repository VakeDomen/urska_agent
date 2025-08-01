import json
from pathlib import Path

PROGRESS_FILE = Path("progress_pre.json")

def correct_progress_file():
    if PROGRESS_FILE.exists():
        with open(PROGRESS_FILE, 'r') as f:
            done = set(json.load(f))

        corrected_done = set()
        for path in done:
            # Convert the path to a Path object and normalize it
            path_obj = Path(path)
            # Extract the relevant parts of the path
            parts = [part for part in path_obj.parts if part not in ['resources', 'english']][-4:]
            corrected_path = str(Path(*parts))
            corrected_done.add(corrected_path)

        with open(PROGRESS_FILE, 'w') as f:
            json.dump(sorted(corrected_done), f, indent=2)

if __name__ == "__main__":
    correct_progress_file()
