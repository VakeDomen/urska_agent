#!/usr/bin/env python3
"""
pdf_to_markdown.py: Convert PDF(s) directly to structured Markdown using the Marker library
"""
import argparse
from pathlib import Path
from typing import Iterable
from marker.converters.pdf import PdfConverter
from marker.models import create_model_dict
from marker.output import text_from_rendered

import os

from docx import Document

def convert_docx_to_markdown(docx_path: Path, output_dir: Path) -> None:
    """
    Convert a DOCX to Markdown (basic text + paragraphs).
    """
    output_dir.mkdir(parents=True, exist_ok=True)

    doc = Document(docx_path)
    lines = []
    for para in doc.paragraphs:
        text = para.text.strip()
        if text:
            lines.append(text)

    markdown = "\n\n".join(lines)

    md_path = output_dir / f"{docx_path.stem}.md"
    md_path.write_text(markdown, encoding="utf-8")
    print(f"Written Markdown to {md_path}")

def convert_to_markdown(file_path: Path, output_dir: Path, use_llm: bool) -> None:
    if file_path.suffix.lower() == ".pdf":
        convert_pdf_to_markdown(file_path, output_dir, use_llm)
    elif file_path.suffix.lower() == ".docx":
        convert_docx_to_markdown(file_path, output_dir)
    else:
        print(f"Skipping unsupported file: {file_path}")


def convert_pdf_to_markdown(
    pdf_path: Path,
    output_dir: Path,
    use_llm: bool
) -> None:
    """
    Convert a PDF to Markdown and extract images using Marker.

    Args:
        pdf_path: Path to the input PDF file.
        output_dir: Directory where output .md and images will be saved.
        use_llm: Whether to boost accuracy using an LLM (requires additional setup).
    """
    output_dir.mkdir(parents=True, exist_ok=True)

    converter = PdfConverter(artifact_dict=create_model_dict())
    rendered = converter(str(pdf_path))
    markdown, metadata, images = text_from_rendered(rendered)

    # Write Markdown
    md_filename = pdf_path.stem + ".md"
    md_path = output_dir / md_filename
    md_path.write_text(markdown, encoding="utf-8")
    print(f"Written Markdown to {md_path}")

    # Write extracted images
    for idx, img in enumerate(images, start=1):
        img_name = f"{pdf_path.stem}_image_{idx}.png"
        img_path = output_dir / img_name
        with open(img_path, "wb") as f:
            f.write(img)
    if images:
        print(f"Extracted {len(images)} image(s) to {output_dir}")



def iter_docs(root: Path) -> Iterable[Path]:
    """
    Yield all supported files (PDF, DOCX) under root, at any depth.
    """
    if root.is_file() and root.suffix.lower() in [".pdf", ".docx"]:
        yield root
    elif root.is_dir():
        for ext in ("*.pdf", "*.docx"):
            yield from root.rglob(ext)



def process_input(input_path: Path, output_dir: Path, use_llm: bool) -> None:
    """
    Process a single PDF/DOCX or every supported file under a directory recursively.
    Preserves the input directory structure inside output_dir.
    """
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
    parser = argparse.ArgumentParser(
        description="Convert PDF(s) to Markdown using Marker library"
    )
    parser.add_argument(
        "input",
        type=Path,
        help="Path to a PDF file or a directory containing PDFs"
    )
    parser.add_argument(
        "-o", "--output-dir",
        type=Path,
        default=Path("."),
        help="Directory to save the output Markdown and images"
    )
    parser.add_argument(
        "--use-llm",
        action="store_true",
        help="Enhance conversion accuracy using an LLM"
    )
    args = parser.parse_args()

    process_input(args.input, args.output_dir, args.use_llm)


if __name__ == "__main__":
    main()
