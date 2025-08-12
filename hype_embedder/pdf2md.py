#!/usr/bin/env python3
"""
pdf_to_markdown.py: Convert a PDF directly to structured Markdown using the Marker library
"""
import os
import argparse
from pathlib import Path

from marker.converters.pdf import PdfConverter
from marker.models import create_model_dict
from marker.output import text_from_rendered


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

    # Initialize converter
    converter = PdfConverter(
        artifact_dict=create_model_dict()
    )
    # Perform conversion
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


def main():
    parser = argparse.ArgumentParser(
        description="Convert PDF to Markdown using Marker library"
    )
    parser.add_argument(
        "pdf",
        type=Path,
        help="Path to the PDF file to convert"
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

    convert_pdf_to_markdown(
        pdf_path=args.pdf,
        output_dir=args.output_dir,
        use_llm=args.use_llm
    )


if __name__ == "__main__":
    main()
