import json
import shutil
import threading
import zipfile

import pytest
from docx import Document
from PIL import Image
from pydantic import ValidationError

from notegen.errors import NoteGenError
from notegen.events import Activity
from notegen.export import clean_xml, export_docx
from notegen.imaging import prepare
from notegen.layout import extract_layout, ruled_tables
from notegen.models import Block, Box, Merge, PageResult, Settings, local_endpoint, select_pages
from notegen.ocr import Tesseract, Word, parse_tsv, run_process
from notegen.pdf import PdfSource
from notegen.storage import Project, atomic_json, file_hash, safe_image


def test_contracts_validate_structure():
    with pytest.raises(ValidationError):
        Block(kind="table", rows=[["a"], ["b", "c"]])
    with pytest.raises(ValidationError):
        Block(kind="figure")
    with pytest.raises(ValidationError):
        Block(kind="table", rows=[["a", "b"]], merges=[Merge(row=0, col=1, colspan=2, rowspan=1)])
    with pytest.raises(ValidationError):
        Block(
            kind="table",
            rows=[["a", "b"], ["c", "d"]],
            merges=[
                Merge(row=0, col=0, rowspan=2, colspan=1),
                Merge(row=0, col=0, rowspan=1, colspan=2),
            ],
        )
    with pytest.raises(ValidationError):
        Box(left=0.5, right=0.1, top=0, bottom=1)
    with pytest.raises(ValidationError):
        Settings(language="eng;shutdown")


@pytest.mark.parametrize(
    "endpoint",
    [
        "https://example.com:8080",
        "http://localhost:80",
        "http://127.0.0.1:80/x",
        "http://127.0.0.1:80?x=y",
        "http://user:pw@127.0.0.1:80",
        "http://0.0.0.0:80",
        "http://127.0.0.1:99999",
    ],
)
def test_endpoints_block_external_or_ambiguous_destinations(endpoint):
    with pytest.raises(NoteGenError, match="NG203"):
        local_endpoint(endpoint)


def test_valid_endpoints_and_page_ranges():
    assert local_endpoint("http://[::1]:8080/") == "http://[::1]:8080"
    assert select_pages("3, 1-2, 2", 5) == [1, 2, 3]
    for spec in ("0", "2-1", "6", "1--3", "1,", "abc"):
        with pytest.raises(NoteGenError):
            select_pages(spec, 5)
    assert Settings().fingerprint("a") != Settings().fingerprint("b")


def test_raster_pdf_bounds_and_invalid_input(scanned_pdf, tmp_path):
    path, _ = scanned_pdf
    with PdfSource(path) as source:
        assert source.count == 1
        image = source.render(1, dpi=400, max_pixels=1_000_000)
        assert image.width * image.height <= 1_002_000
        with pytest.raises(NoteGenError):
            source.render(2)
    bad = tmp_path / "broken.pdf"
    bad.write_bytes(b"not a PDF")
    with pytest.raises(NoteGenError, match="NG100"):
        PdfSource(bad)


def test_prepare_preserves_color_and_blank():
    image = Image.new("RGB", (200, 300), (255, 255, 255))
    output = prepare(image)
    assert output.image.mode == "RGB" and output.ocr.mode == "L"
    assert output.image.size == image.size and output.angle == 0


def test_tables_rebuilt_from_word_boxes(scanned_pdf):
    _, image = scanned_pdf
    words = [
        Word("Item", 110, 430, 80, 30, 0.95),
        Word("Amount", 630, 430, 100, 30, 0.95),
        Word("Sales", 110, 530, 80, 30, 0.95),
        Word("1250", 630, 530, 100, 30, 0.95),
    ]
    tables = ruled_tables(image, words)
    assert len(tables) == 1
    assert tables[0].rows == [["Item", "Amount"], ["Sales", "1250"]]
    content = extract_layout(image, words)
    assert [b.kind for b in content.blocks].count("table") == 1
    assert not any(b.kind == "text" for b in content.blocks)


def test_tsv_malformed_rows():
    data = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n"
    data += "5\t1\t1\t1\t1\t1\t10\t20\t30\t40\t96\tHello\n"
    data += "5\t1\t1\t1\t1\t2\t10\t20\t30\t40\tbad\tignored\n"
    assert parse_tsv(data)[0].text == "Hello"
    assert len(parse_tsv(data)) == 1


@pytest.mark.ocr
@pytest.mark.skipif(not shutil.which("tesseract"), reason="Tesseract not installed")
def test_real_scanned_pdf_ocr(scanned_pdf):
    path, _ = scanned_pdf
    with PdfSource(path) as pdf:
        image = pdf.render(1, dpi=150)
    prepared = prepare(image)
    words = Tesseract().recognize(prepared.ocr)
    text = " ".join(w.text for w in words)
    assert "Revenue" in text and "1250" in text and "computer" in text
    content = extract_layout(prepared.image, words)
    assert any(
        b.kind == "table" and any("1250" in c for r in b.rows for c in r) for b in content.blocks
    )


def test_subprocess_cancellation():
    import sys

    cancel = threading.Event()
    cancel.set()
    with pytest.raises(NoteGenError, match="NG900"):
        run_process([sys.executable, "-c", "import time; time.sleep(10)"], cancel=cancel)


def test_project_resume_mismatch_and_path_safety(project_root):
    project = Project(project_root, "abc", "def", [1])
    Image.new("RGB", (40, 40), "white").save(project.image_path(1))
    page = PageResult(
        page=1,
        engine="tesseract",
        image="page-00001.png",
        blocks=[Block(kind="text", text="Hello", confidence=0.99)],
    )
    project.save(page)
    assert Project(project_root, "abc", "def", [1]).load(1) == page
    with pytest.raises(NoteGenError, match="NG500"):
        Project(project_root, "abc", "changed", [1])
    with pytest.raises(NoteGenError):
        safe_image(project_root, "../../elsewhere.png")
    atomic_json(project_root / "page-00001.json", {"broken": True})
    assert project.load(1) is None
    assert len(file_hash(project.manifest_path)) == 64


def test_docx_editable_table_merges_figures_and_no_overwrite(project_root, tmp_path):
    Image.new("RGB", (800, 1000), "white").save(project_root / "page-00001.png")
    page = PageResult(
        page=1,
        engine="vision",
        image="page-00001.png",
        warnings=["Check scan"],
        blocks=[
            Block(kind="heading", text="Revenue"),
            Block(kind="text", text="Income\x00 with control character removed", confidence=0.9),
            Block(
                kind="table",
                rows=[["Year", ""], ["2026", "1250"]],
                header_rows=1,
                merges=[Merge(row=0, col=0, rowspan=1, colspan=2)],
            ),
            Block(
                kind="figure",
                box=Box(left=0.1, top=0.2, right=0.7, bottom=0.8),
                description="An arrow connects A to B.",
                interpretation="The sequence may show a process.",
            ),
        ],
    )
    path = export_docx([page], project_root, tmp_path / "result.docx")
    doc = Document(path)
    assert len(doc.tables) == 1 and doc.tables[0].cell(1, 1).text == "1250"
    assert len(doc.inline_shapes) == 1
    text = "\n".join(p.text for p in doc.paragraphs)
    assert "Observed:" in text and "REVIEW NEEDED" in text
    with zipfile.ZipFile(path) as z:
        xml = z.read("word/document.xml").decode()
        assert "gridSpan" in xml and "tblHeader" in xml and "An arrow connects" in xml
    with pytest.raises(NoteGenError, match="NG401"):
        export_docx([page], project_root, path)
    assert clean_xml("a\x00b") == "ab"


def test_live_activity_has_machine_readable_codes(tmp_path):
    received = []
    log = tmp_path / "activity.jsonl"
    Activity(log, received.append).emit("ocr", "Page recognized", page=1, total=2, code="NG111")
    assert received[0].page == 1
    assert json.loads(log.read_text())["code"] == "NG111"


def test_vertical_table_merges_survive_docx_export(project_root, tmp_path):
    page = PageResult(
        page=1,
        engine="test",
        image="page-00001.png",
        blocks=[
            Block(
                kind="table",
                rows=[["Group A", "10"], ["", "20"]],
                merges=[Merge(row=0, col=0, rowspan=2, colspan=1)],
            )
        ],
    )
    path = export_docx([page], project_root, tmp_path / "vertical.docx")
    with zipfile.ZipFile(path) as z:
        assert b'w:vMerge w:val="restart"' in z.read("word/document.xml")
    table = Document(path).tables[0]
    assert table.cell(1, 0).text == "Group A" and table.cell(1, 1).text == "20"
