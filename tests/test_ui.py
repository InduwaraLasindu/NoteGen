import json
import shutil

import pytest
from docx import Document
from PIL import Image
from PySide6.QtCore import Qt
from PySide6.QtWidgets import QTableWidgetSelectionRange

from notegen.app import MainWindow
from notegen.errors import NoteGenError
from notegen.events import Event
from notegen.models import Block, Box, PageResult
from notegen.review import ReviewPanel
from notegen.storage import Project
from notegen.theme import STYLE


@pytest.fixture
def window(qtbot, qapp, tmp_path):
    qapp.setStyle("Fusion")
    qapp.setStyleSheet(STYLE)
    value = MainWindow(tmp_path / "outputs", persist=False, telemetry=False)
    qtbot.addWidget(value)
    value.show()
    return value


@pytest.fixture
def review(qtbot, project_root):
    Image.new("RGB", (800, 1000), "white").save(project_root / "page-00001.png")
    page = PageResult(
        page=1,
        engine="test",
        image="page-00001.png",
        blocks=[
            Block(kind="text", text="Original text", confidence=0.8),
            Block(kind="table", rows=[["Year", "Amount"], ["2026", "1250"]], header_rows=1),
            Block(
                kind="figure",
                box=Box(left=0.1, top=0.1, right=0.8, bottom=0.8),
                description="A connects to B",
                interpretation="A process may be shown",
                uncertain=True,
            ),
        ],
    )
    widget = ReviewPanel()
    qtbot.addWidget(widget)
    widget.load_project(project_root, [page])
    widget.show()
    return widget


def test_queue_dedup_settings_and_keyboard_shortcut(window, scanned_pdf, qtbot):
    path, _ = scanned_pdf
    window.add_files([str(path), str(path), "/missing/file.pdf"])
    assert len(window.queue) == 1 and window.file_list.count() == 1
    assert not window.empty_hint.isVisible()
    window.mode.setCurrentIndex(window.mode.findData("fast"))
    window.handwriting.setChecked(True)
    assert window.settings().mode == "balanced" and window.settings().handwriting
    qtbot.keyClick(window, Qt.Key.Key_L, Qt.KeyboardModifier.ControlModifier)
    assert window.tabs.currentIndex() == 2
    window.running_queue = True
    window.file_list.setCurrentRow(0)
    window.remove_selected()
    assert len(window.queue) == 1
    window.running_queue = False
    window.remove_selected()
    assert not window.queue


def test_live_progress_filter_and_actionable_errors(window):
    window.on_event(Event("ocr", "Recognizing", page=1, done=0, total=2))
    assert window.page_progress.value() == 35
    assert 0 < window.overall.value() < 500
    window.on_event(Event("page_done", "Saved", page=1, done=1, total=2))
    assert window.overall.value() == 500
    window.on_event(Event("recovery", "Retry", level="warning", code="NG202"))
    window.log_filter.setCurrentIndex(1)
    assert "NG202" in window.log_view.toPlainText()
    assert "Recognizing" not in window.log_view.toPlainText()
    window.present_error(NoteGenError("NG200"))
    assert "Fix:" in window.error_box.toPlainText()
    assert "Start your local" in window.error_box.toPlainText()
    assert (window.output_dir / ".notegen" / "session.jsonl").exists()


def test_text_edits_autosave_on_region_change_and_export(review, project_root, tmp_path):
    review.text.setPlainText("Corrected amount is 1250 <literal>")
    assert review.dirty
    review.block_list.setCurrentRow(1)
    assert not review.dirty
    saved = json.loads((project_root / "page-00001.json").read_text())
    assert saved["blocks"][0]["text"] == "Corrected amount is 1250 <literal>"
    path = review.export_to(tmp_path / "reviewed.docx")
    assert "Corrected amount" in " ".join(p.text for p in Document(path).paragraphs)


def test_table_edit_merge_and_split_are_real_word_cells(review, tmp_path):
    review.block_list.setCurrentRow(1)
    review.table.item(1, 1).setText("1500")
    assert review.save_block()
    review.table.setRangeSelected(QTableWidgetSelectionRange(0, 0, 0, 1), True)
    review.merge_selection()
    block = review.pages[0].blocks[1]
    assert block.rows[1][1] == "1500" and block.merges[0].colspan == 2
    assert block.rows[0][0] == "Year Amount" and block.rows[0][1] == ""
    path = review.export_to(tmp_path / "merged.docx")
    assert Document(path).tables[0].cell(1, 1).text == "1500"
    review.split_merges()
    assert not review.pages[0].blocks[1].merges


def test_figure_review_and_page_confirmation(review):
    review.block_list.setCurrentRow(2)
    review.observed.setPlainText("The arrow points from A to B.")
    review.meaning.setPlainText("Likely the first two steps of a sequence.")
    review.uncertain.setChecked(False)
    review.reviewed.setChecked(True)  # Autosave the region before confirming the page.
    assert review.pages[0].reviewed and review.reviewed.isChecked()
    assert not review.pages[0].needs_review()
    assert review.pages[0].blocks[2].description == "The arrow points from A to B."
    review.caption.setPlainText("Figure 1")
    review.save_block()
    assert not review.pages[0].reviewed and not review.reviewed.isChecked()


def test_saved_project_reopens_for_review(window, project_root):
    project = Project(project_root, "a", "b", [1])
    Image.new("RGB", (100, 100), "white").save(project.image_path(1))
    project.save(
        PageResult(
            page=1,
            engine="tesseract",
            image="page-00001.png",
            blocks=[Block(kind="text", text="Recovered page", confidence=0.99)],
        )
    )
    window.load_saved_project(project_root)
    assert window.review.text.toPlainText() == "Recovered page"


@pytest.mark.ocr
@pytest.mark.skipif(not shutil.which("tesseract"), reason="Tesseract not installed")
def test_desktop_worker_real_conversion(window, scanned_pdf, qtbot):
    path, _ = scanned_pdf
    window.mode.setCurrentIndex(window.mode.findData("fast"))
    window.dpi.setValue(150)
    window.add_files([str(path)])
    qtbot.mouseClick(window.start_button, Qt.MouseButton.LeftButton)
    qtbot.waitUntil(lambda: window.worker is None and not window.running_queue, timeout=20000)
    assert window.queue[0].state == "Completed"
    assert window.last_output.is_file()
    assert window.review.page_list.count() == 1
    assert window.overall.value() == 1000
    assert "Word" in window.stage_label.text()
    assert window.start_button.isEnabled() and not window.cancel_button.isEnabled()


def test_small_desktop_layout_keeps_controls_available(window, qtbot):
    window.resize(1000, 700)
    qtbot.wait(30)
    assert window.sidebar.width() >= 270
    assert window.tabs.width() > 500
    assert window.start_button.isVisible()
    assert window.file_list.minimumHeight() >= 160


def test_table_merge_invalidates_the_visible_review_confirmation(review):
    review.block_list.setCurrentRow(1)
    review.reviewed.setChecked(True)
    assert review.pages[0].reviewed
    review.table.setRangeSelected(QTableWidgetSelectionRange(0, 0, 0, 1), True)
    review.merge_selection()
    assert not review.pages[0].reviewed and not review.reviewed.isChecked()
    review.reviewed.setChecked(True)
    review.split_merges()
    assert not review.pages[0].reviewed and not review.reviewed.isChecked()


def test_completed_queue_item_reopens_its_own_review(window, scanned_pdf, project_root):
    path, _ = scanned_pdf
    window.add_files([str(path)])
    project = Project(project_root, "a", "b", [1])
    Image.new("RGB", (100, 100), "white").save(project.image_path(1))
    project.save(
        PageResult(
            page=1,
            engine="test",
            image="page-00001.png",
            blocks=[Block(kind="text", text="Selected document review", confidence=0.99)],
        )
    )
    window.queue[0].state = "Completed"
    window.queue[0].project = project_root
    window.file_list.setCurrentRow(0)
    window.open_selected_job()
    assert window.review.text.toPlainText() == "Selected document review"
    assert window.tabs.currentIndex() == 1
