"""Editable DOCX export with source figures, real tables and visible uncertainty."""

import io
import os
import tempfile
from pathlib import Path

import psutil
from docx import Document
from docx.enum.text import WD_BREAK
from docx.oxml import OxmlElement
from docx.oxml.ns import qn
from docx.shared import Inches, Pt, RGBColor
from PIL import Image

from .errors import NoteGenError
from .models import PageResult
from .storage import safe_image


def clean_xml(text: str) -> str:
    return "".join(
        c
        for c in text
        if c in "\t\n\r"
        or 0x20 <= ord(c) <= 0xD7FF
        or 0xE000 <= ord(c) <= 0xFFFD
        or 0x10000 <= ord(c) <= 0x10FFFF
    )


def export_docx(
    pages: list[PageResult],
    root: Path,
    destination: Path,
    title: str = "Converted document",
    cancel=None,
) -> Path:
    destination = Path(destination)
    if destination.suffix.lower() != ".docx":
        raise NoteGenError("NG400", "Choose a .docx output")
    if destination.exists():
        raise NoteGenError("NG401")
    destination.parent.mkdir(parents=True, exist_ok=True)
    doc = Document()
    style = doc.styles["Normal"]
    style.font.name = "Calibri"
    style.font.size = Pt(11)
    # Explicit complex-script font improves editable Sinhala/Tamil when fonts are installed.
    style.element.get_or_add_rPr().rFonts.set(qn("w:cs"), "Nirmala UI")
    doc.core_properties.title = title
    doc.core_properties.subject = "Local OCR conversion; uncertain content requires source review"
    doc.sections[0].header.paragraphs[0].text = clean_xml(title)
    footer = doc.sections[0].footer.paragraphs[0]
    footer.text = "NoteGen • "
    field = OxmlElement("w:fldSimple")
    field.set(qn("w:instr"), "PAGE")
    footer._p.append(field)
    for index, page in enumerate(pages):
        if cancel and cancel.is_set():
            raise NoteGenError("NG900")
        if (
            psutil.virtual_memory().available < 2**30
            or psutil.Process().memory_info().rss > 4 * 2**30
        ):
            raise NoteGenError("NG300", "Export memory safety margin reached; split the page range")
        if index:
            doc.add_paragraph().add_run().add_break(WD_BREAK.PAGE)
        p = doc.add_paragraph(f"Source page {page.page}", style="Caption")
        p.paragraph_format.keep_with_next = True
        if page.needs_review():
            p = doc.add_paragraph("REVIEW NEEDED — compare uncertain content with the source scan.")
            p.runs[0].font.color.rgb = RGBColor.from_string("9C5A00")
        for warning in page.warnings:
            doc.add_paragraph(clean_xml(warning), style="Caption")
        for block in page.blocks:
            if block.kind == "table":
                table = doc.add_table(rows=len(block.rows), cols=len(block.rows[0]))
                table.style = "Table Grid"
                for r, row in enumerate(block.rows):
                    for c, text in enumerate(row):
                        table.cell(r, c).text = clean_xml(text)
                        if r < block.header_rows:
                            for run in table.cell(r, c).paragraphs[0].runs:
                                run.bold = True
                    if r < block.header_rows:
                        repeat = OxmlElement("w:tblHeader")
                        table.rows[r]._tr.get_or_add_trPr().append(repeat)
                for merge in block.merges:
                    merged = table.cell(merge.row, merge.col).merge(
                        table.cell(merge.row + merge.rowspan - 1, merge.col + merge.colspan - 1)
                    )
                    # Do not concatenate empty placeholder paragraphs as blank lines.
                    merged.text = clean_xml(
                        "\n".join(
                            block.rows[r][c]
                            for r in range(merge.row, merge.row + merge.rowspan)
                            for c in range(merge.col, merge.col + merge.colspan)
                            if block.rows[r][c]
                        )
                    )
                    if merge.row < block.header_rows:
                        for paragraph in merged.paragraphs:
                            for run in paragraph.runs:
                                run.bold = True
                doc.add_paragraph()
            elif block.kind == "figure":
                path = safe_image(root, page.image)
                with Image.open(path) as im:
                    crop = im.crop(block.box.pixels(im.size)).convert("RGB")
                    crop.thumbnail((1800, 2400), Image.Resampling.LANCZOS)
                    data = io.BytesIO()
                    crop.save(data, format="JPEG", quality=94)
                    data.seek(0)
                    shape = doc.add_picture(data, width=Inches(min(6, crop.width / 160)))
                    shape._inline.docPr.set(
                        "descr", clean_xml(block.description or "Source diagram")
                    )
                if block.text:
                    doc.add_paragraph(clean_xml(block.text), style="Caption")
                doc.add_paragraph(
                    "Observed: " + clean_xml(block.description or "Description unavailable.")
                )
                if block.interpretation:
                    doc.add_paragraph(
                        "Contextual interpretation: " + clean_xml(block.interpretation)
                    )
                doc.add_paragraph(
                    "AI descriptions and interpretations require source verification.",
                    style="Caption",
                )
            else:
                style_name = {"heading": "Heading 2", "list": "List Bullet"}.get(block.kind)
                p = doc.add_paragraph(clean_xml(block.text), style=style_name)
                if block.uncertain:
                    p.add_run(" [uncertain]").font.color.rgb = RGBColor.from_string("9C5A00")
                if block.kind == "equation":
                    p.add_run(" [editable transcription; verify symbols]").italic = True
    fd, tmp = tempfile.mkstemp(dir=destination.parent, prefix=".notegen-", suffix=".docx")
    os.close(fd)
    try:
        doc.save(tmp)
        # Reserve without overwrite, including races between a queued job and another app.
        with destination.open("xb"):
            pass
        try:
            os.replace(tmp, destination)
        except OSError:
            destination.unlink(missing_ok=True)
            raise
        return destination
    except FileExistsError as exc:
        raise NoteGenError("NG401") from exc
    except OSError as exc:
        raise NoteGenError("NG400", type(exc).__name__) from exc
    finally:
        Path(tmp).unlink(missing_ok=True)
