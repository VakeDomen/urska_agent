#!/usr/bin/env python3
import argparse
from pathlib import Path
from typing import Iterable, Tuple, List

from marker.converters.pdf import PdfConverter
from marker.models import create_model_dict
from marker.output import text_from_rendered

# try Mammoth for DOCX
try:
    import mammoth  # type: ignore
    HAS_MAMMOTH = True
except ImportError:
    HAS_MAMMOTH = False

def convert_pdf_to_markdown(pdf_path: Path, output_dir: Path, use_llm: bool) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    converter = PdfConverter(artifact_dict=create_model_dict())
    rendered = converter(str(pdf_path))
    markdown, metadata, images = text_from_rendered(rendered)

    md_path = output_dir / f"{pdf_path.stem}.md"
    md_path.write_text(markdown, encoding="utf-8")
    print(f"Written Markdown to {md_path}")

    for idx, img in enumerate(images, start=1):
        img_path = output_dir / f"{pdf_path.stem}_image_{idx}.png"
        with open(img_path, "wb") as f:
            f.write(img)
    if images:
        print(f"Extracted {len(images)} image(s) to {output_dir}")

def _mammoth_image_handler(output_dir: Path, stem: str):
    # returns a function Mammoth uses to write images and return md links
    def handler(image):
        ext = image.content_type.split("/")[-1] or "png"
        existing = list(output_dir.glob(f"{stem}_image_*.{ext}"))
        next_idx = len(existing) + 1
        filename = f"{stem}_image_{next_idx}.{ext}"
        path = output_dir / filename
        with path.open("wb") as f:
            f.write(image.read())
        return {"src": filename}
    return handler

def convert_docx_to_markdown(docx_path: Path, output_dir: Path) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)

    if not HAS_MAMMOTH:
        raise RuntimeError(
            "mammoth is not installed. Install it with: pip install mammoth"
        )

    with open(docx_path, "rb") as f:
        result = mammoth.convert_to_markdown(
            f,
            convert_image=mammoth.images.inline(_mammoth_image_handler(output_dir, docx_path.stem))
        )
    markdown = result.value
    messages = result.messages  # warnings, missing styles, etc.

    md_path = output_dir / f"{docx_path.stem}.md"
    md_path.write_text(markdown, encoding="utf-8")
    print(f"Written Markdown to {md_path}")
    if messages:
        for m in messages:
            print(f"[mammoth] {m.type}: {m.message}")

def convert_to_markdown(file_path: Path, output_dir: Path, use_llm: bool) -> None:
    suf = file_path.suffix.lower()
    if suf == ".pdf":
        convert_pdf_to_markdown(file_path, output_dir, use_llm)
    elif suf == ".docx":
        convert_docx_to_markdown(file_path, output_dir)
    else:
        print(f"Skipping unsupported file: {file_path}")

def iter_docs(root: Path) -> Iterable[Path]:
    if root.is_file() and root.suffix.lower() in [".pdf", ".docx"]:
        yield root
    elif root.is_dir():
        for ext in ("*.pdf", "*.docx"):
            yield from root.rglob(ext)

def process_input(input_path: Path, output_dir: Path, use_llm: bool) -> None:
    base = input_path if input_path.is_dir() else input_path.parent
    total = 0
    errors = 0

    for file_path in iter_docs(input_path):
        rel_dir = file_path.parent.relative_to(base)
        target_dir = output_dir / rel_dir
        try:
            convert_to_markdown(file_path, target_dir, use_llm)
            total += 1
        except Exception as e:
            errors += 1
            print(f"Error converting {file_path}: {e}")

    if total == 0:
        print(f"No PDFs or DOCX files found under {input_path}")
    else:
        print(f"Done. Converted {total} file(s). Errors: {errors}")

def main():
    parser = argparse.ArgumentParser(description="Convert PDFs and DOCX to Markdown")
    parser.add_argument("input", type=Path, help="Path to a file or directory")
    parser.add_argument("-o", type=Path, dest="output_dir", default=Path("."), help="Output directory")
    parser.add_argument("--use-llm", action="store_true", help="Boost PDF accuracy with an LLM if configured")
    args = parser.parse_args()
    process_input(args.input, args.output_dir, args.use_llm)

if __name__ == "__main__":
    main()
