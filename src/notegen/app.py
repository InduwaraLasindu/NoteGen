"""Native Windows desktop: queued conversions, live activity and source-aware review."""

import json
import sys
import time
from dataclasses import dataclass
from pathlib import Path

from PySide6.QtCore import QSettings, QStandardPaths, QThread, QTimer, QUrl, Signal
from PySide6.QtGui import QDesktopServices, QFont, QKeySequence, QShortcut
from PySide6.QtWidgets import (
    QApplication,
    QCheckBox,
    QComboBox,
    QFileDialog,
    QFormLayout,
    QFrame,
    QHBoxLayout,
    QLineEdit,
    QListWidgetItem,
    QMainWindow,
    QPlainTextEdit,
    QProgressBar,
    QPushButton,
    QScrollArea,
    QSpinBox,
    QSplitter,
    QTabWidget,
    QTextBrowser,
    QVBoxLayout,
    QWidget,
)

from . import __version__
from .cli import doctor
from .errors import classify
from .events import Activity, Event
from .models import Settings
from .pipeline import Control, convert
from .resources import Resources, snapshot
from .review import ReviewPanel
from .storage import Project
from .theme import STYLE
from .widgets import Metric, PdfList, label


@dataclass
class QueueItem:
    source: Path
    state: str = "Waiting"
    output: Path | None = None
    project: Path | None = None


class Task(QThread):
    result = Signal(object)
    failed = Signal(object)

    def __init__(self, function, parent=None):
        super().__init__(parent)
        self.function = function

    def run(self):
        try:
            self.result.emit(self.function())
        except Exception as exc:
            self.failed.emit(classify(exc))


class ConversionWorker(QThread):
    event = Signal(object)
    result = Signal(object)
    failed = Signal(object)

    def __init__(self, source, output, settings, parent=None):
        super().__init__(parent)
        self.source, self.output, self.settings = source, output, settings
        self.control = Control()

    def run(self):
        try:
            result = convert(
                self.source,
                self.output,
                self.settings,
                Activity(callback=self.event.emit),
                self.control,
            )
            self.result.emit(result)
        except Exception as exc:
            self.failed.emit(classify(exc))


class MainWindow(QMainWindow):
    def __init__(self, output_dir: Path | None = None, persist=True, telemetry=True):
        super().__init__()
        self.setWindowTitle(f"NoteGen {__version__} • Local document intelligence")
        self.resize(1440, 920)
        self.setMinimumSize(980, 680)
        self.persist_preferences = persist
        self.preferences = QSettings("NoteGen", "NoteGen")
        default_docs = QStandardPaths.writableLocation(
            QStandardPaths.StandardLocation.DocumentsLocation
        )
        self.output_dir = output_dir or Path(
            self.preferences.value("output_dir", str(Path(default_docs) / "NoteGen"))
        )
        self.queue: list[QueueItem] = []
        self.active_index = -1
        self.worker = None
        self.queue_settings = None
        self.running_queue = False
        self.close_requested = False
        self.telemetry_task = None
        self.doctor_task = None
        self.events: list[Event] = []
        self.started = 0.0
        self.last_done = 0
        self.total = 0
        self.last_output = None
        self.last_project = None
        self.current_error = None
        self.session_activity = Activity(self.output_dir / ".notegen" / "session.jsonl")
        outer = QWidget()
        self.setCentralWidget(outer)
        layout = QVBoxLayout(outer)
        layout.setContentsMargins(24, 18, 24, 12)
        layout.setSpacing(16)
        header = QHBoxLayout()
        brand = QVBoxLayout()
        brand.addWidget(label("NoteGen", "brand"))
        brand.addWidget(label("YOUR SCANS. CLEARER NOTES.", "eyebrow"))
        header.addLayout(brand)
        header.addStretch()
        header.addWidget(label("●  LOCAL PROCESSING", "eyebrow"))
        self.resource_label = label(
            "Checking hardware…" if telemetry else "Local processing · Offline ready", "muted"
        )
        header.addSpacing(20)
        header.addWidget(self.resource_label)
        layout.addLayout(header)
        self.main_splitter = QSplitter()
        layout.addWidget(self.main_splitter, 1)
        self.sidebar = self.build_sidebar()
        self.main_splitter.addWidget(self.sidebar)
        self.tabs = QTabWidget()
        self.tabs.addTab(self.build_workspace(), "Workspace")
        self.review = ReviewPanel()
        self.tabs.addTab(self.review, "Review and correct")
        self.tabs.addTab(self.build_activity(), "Live activity")
        self.tabs.addTab(self.build_setup(), "Setup and diagnostics")
        self.main_splitter.addWidget(self.tabs)
        self.main_splitter.setSizes([310, 1080])
        self.statusBar().showMessage("Ready • Add PDFs or drop files into the queue")
        self.review.changed.connect(self.update_review_count)
        for keys, action in [
            ("Ctrl+O", self.choose_files),
            ("Ctrl+Return", self.start_queue),
            ("Ctrl+L", lambda: self.tabs.setCurrentIndex(2)),
        ]:
            QShortcut(QKeySequence(keys), self, activated=action)
        self.clock = QTimer(self)
        self.clock.timeout.connect(self.tick)
        self.clock.start(1000)
        self.resource_timer = QTimer(self)
        self.resource_timer.timeout.connect(self.poll_resources)
        if telemetry:
            self.resource_timer.start(5000)
            QTimer.singleShot(0, self.poll_resources)
        self.restore_preferences()

    def build_sidebar(self):
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setMinimumWidth(270)
        scroll.setMaximumWidth(360)
        inner = QFrame()
        inner.setObjectName("panel")
        content = QVBoxLayout(inner)
        content.setContentsMargins(18, 20, 18, 18)
        content.setSpacing(13)
        content.addWidget(label("CONVERSION SETTINGS", "eyebrow"))
        self.settings_form = QWidget()
        form = QFormLayout(self.settings_form)
        form.setContentsMargins(0, 0, 0, 0)
        form.setRowWrapPolicy(QFormLayout.RowWrapPolicy.WrapAllRows)
        form.setSpacing(10)
        self.mode = QComboBox()
        for name, data in [
            ("Balanced · selective AI", "balanced"),
            ("Thorough · AI on every page", "thorough"),
            ("Fast · printed OCR", "fast"),
        ]:
            self.mode.addItem(name, data)
        self.handwriting = QCheckBox("Expect handwriting")
        self.handwriting.setToolTip(
            "Requires local vision AI. Terrible handwriting may remain illegible."
        )
        self.language = QLineEdit("eng")
        self.language.setPlaceholderText("eng / eng+sin / eng+tam")
        self.language.setToolTip(
            "Tesseract language codes. The matching language files must be installed."
        )
        self.dpi = QSpinBox()
        self.dpi.setRange(100, 400)
        self.dpi.setValue(240)
        self.pages_input = QLineEdit()
        self.pages_input.setPlaceholderText("All pages, or 1-4,7,9")
        self.context = QPlainTextEdit()
        self.context.setMaximumHeight(85)
        self.context.setPlaceholderText("Optional subject: e.g. accounting notes on leases")
        self.auto_rotate = QCheckBox("Correct page orientation")
        self.auto_rotate.setChecked(True)
        self.deskew = QCheckBox("Straighten small scan skew")
        self.deskew.setChecked(True)
        form.addRow("Quality profile", self.mode)
        form.addRow(self.handwriting)
        form.addRow("Printed text languages", self.language)
        form.addRow("Render DPI", self.dpi)
        form.addRow("Page range", self.pages_input)
        form.addRow("Document context", self.context)
        form.addRow(self.auto_rotate)
        form.addRow(self.deskew)
        content.addWidget(self.settings_form)
        content.addWidget(label("SAVE TO", "eyebrow"))
        self.output_label = label(str(self.output_dir), "muted", True)
        self.output_label.setToolTip(str(self.output_dir))
        content.addWidget(self.output_label)
        self.choose_output_button = QPushButton("Choose output folder")
        self.choose_output_button.clicked.connect(self.choose_output)
        content.addWidget(self.choose_output_button)
        content.addStretch()
        self.start_button = QPushButton("Convert queue")
        self.start_button.setObjectName("primary")
        self.start_button.clicked.connect(self.start_queue)
        content.addWidget(self.start_button)
        buttons = QHBoxLayout()
        self.pause_button = QPushButton("Pause")
        self.pause_button.setEnabled(False)
        self.pause_button.clicked.connect(self.toggle_pause)
        self.cancel_button = QPushButton("Cancel")
        self.cancel_button.setObjectName("danger")
        self.cancel_button.setEnabled(False)
        self.cancel_button.clicked.connect(self.cancel_job)
        buttons.addWidget(self.pause_button)
        buttons.addWidget(self.cancel_button)
        content.addLayout(buttons)
        content.addWidget(
            label(
                "Documents stay on this computer.\nNo cloud fallback or telemetry.", "muted", True
            )
        )
        self.handwriting.toggled.connect(self.handwriting_mode)
        self.mode.currentIndexChanged.connect(self.handwriting_mode)
        scroll.setWidget(inner)
        return scroll

    def build_workspace(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.setContentsMargins(16, 16, 0, 0)
        layout.setSpacing(16)
        layout.addWidget(label("From scanned pages to useful notes", "title"))
        layout.addWidget(
            label(
                "Editable text and tables. Diagrams explained in context. Everything local.",
                "muted",
                True,
            )
        )
        metrics = QHBoxLayout()
        self.page_metric = Metric("PAGES COMPLETED", "0 / 0")
        self.review_metric = Metric("PAGES TO REVIEW", "0")
        self.time_metric = Metric("ELAPSED", "00:00")
        metrics.addWidget(self.page_metric)
        metrics.addWidget(self.review_metric)
        metrics.addWidget(self.time_metric)
        layout.addLayout(metrics)
        queue_header = QHBoxLayout()
        queue_header.addWidget(label("DOCUMENT QUEUE", "eyebrow"))
        queue_header.addStretch()
        self.add_button = QPushButton("+ Add PDFs")
        self.add_button.clicked.connect(self.choose_files)
        queue_header.addWidget(self.add_button)
        self.remove_button = QPushButton("Remove selected")
        self.remove_button.clicked.connect(self.remove_selected)
        queue_header.addWidget(self.remove_button)
        layout.addLayout(queue_header)
        self.file_list = PdfList()
        self.file_list.files_dropped.connect(self.add_files)
        self.file_list.itemDoubleClicked.connect(self.open_selected_job)
        layout.addWidget(self.file_list, 1)
        self.empty_hint = label(
            "Drop scanned PDFs here to get started.\nMultiple files are processed one at a time.",
            "muted",
            True,
        )
        layout.addWidget(self.empty_hint)
        progress = QFrame()
        progress.setObjectName("panel")
        pl = QVBoxLayout(progress)
        pl.setContentsMargins(18, 16, 18, 16)
        self.stage_label = label("Ready when you are", "title")
        pl.addWidget(self.stage_label)
        self.overall = QProgressBar()
        self.overall.setRange(0, 1000)
        self.overall.setValue(0)
        self.overall.setTextVisible(False)
        self.overall.setToolTip(
            "Completed page counts are exact. Within-page progress is a stage estimate."
        )
        pl.addWidget(self.overall)
        self.progress_text = label("Document progress · 0%", "muted")
        pl.addWidget(self.progress_text)
        self.page_progress = QProgressBar()
        self.page_progress.setRange(0, 100)
        self.page_progress.setValue(0)
        self.page_progress.setToolTip(
            "Approximate progress through the current page's processing stages."
        )
        pl.addWidget(self.page_progress)
        self.eta_label = label("Time remaining appears after completed pages.", "muted")
        pl.addWidget(self.eta_label)
        layout.addWidget(progress)
        self.error_box = QTextBrowser()
        self.error_box.setMaximumHeight(130)
        self.error_box.hide()
        layout.addWidget(self.error_box)
        actions = QHBoxLayout()
        self.open_button = QPushButton("Open Word file")
        self.open_button.setEnabled(False)
        self.open_button.clicked.connect(self.open_output)
        actions.addWidget(self.open_button)
        self.review_button = QPushButton("Review latest document")
        self.review_button.setEnabled(False)
        self.review_button.clicked.connect(lambda: self.tabs.setCurrentIndex(1))
        actions.addWidget(self.review_button)
        self.open_project_button = QPushButton("Open saved project")
        self.open_project_button.clicked.connect(self.open_project)
        actions.addWidget(self.open_project_button)
        actions.addStretch()
        layout.addLayout(actions)
        return page

    def build_activity(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        toolbar = QHBoxLayout()
        toolbar.addWidget(label("Live conversion activity", "title"))
        toolbar.addStretch()
        self.log_filter = QComboBox()
        self.log_filter.addItems(["All activity", "Warnings & errors", "Errors only"])
        self.log_filter.currentIndexChanged.connect(self.refresh_log)
        toolbar.addWidget(self.log_filter)
        self.copy_error_button = QPushButton("Copy last error")
        self.copy_error_button.clicked.connect(self.copy_error)
        toolbar.addWidget(self.copy_error_button)
        layout.addLayout(toolbar)
        self.log_view = QPlainTextEdit()
        self.log_view.setReadOnly(True)
        self.log_view.setFont(QFont("Consolas", 10))
        self.log_view.document().setMaximumBlockCount(6000)
        layout.addWidget(self.log_view, 1)
        layout.addWidget(
            label(
                "Activity logs are saved locally alongside conversion checkpoints. Recognized text is not logged.",
                "muted",
                True,
            )
        )
        return page

    def build_setup(self):
        page = QWidget()
        layout = QVBoxLayout(page)
        layout.addWidget(label("Local engine setup", "title"))
        help_text = label(
            "1. Install Tesseract for quick printed OCR and your language data.\n\n"
            "2. Download a CUDA build of llama.cpp, a vision GGUF and its matching projector.\n"
            "Recommended starting model: Qwen2.5-VL-7B-Instruct Q4_K_M.\n\n"
            "3. Run scripts/Start-LocalAI.ps1 with those local file paths. It binds to 127.0.0.1.\n\n"
            "4. Run diagnostics below, then add your PDF. Once installed, conversion works offline.\n\n"
            "Fast mode uses CPU OCR. Balanced selects difficult pages. Thorough reads every page with AI.\n"
            "Handwriting requires vision AI. Severely blurred or illegible text may need manual correction.",
            "muted",
            True,
        )
        layout.addWidget(help_text)
        self.engine_settings = QWidget()
        form = QFormLayout(self.engine_settings)
        self.tesseract = QLineEdit()
        self.tesseract.setPlaceholderText(
            "Auto-detect, or C:\\Program Files\\Tesseract-OCR\\tesseract.exe"
        )
        browse = QPushButton("Browse")
        browse.clicked.connect(self.choose_tesseract)
        row = QHBoxLayout()
        row.addWidget(self.tesseract)
        row.addWidget(browse)
        form.addRow("Tesseract executable", row)
        self.endpoint = QLineEdit("http://127.0.0.1:8080")
        self.model = QLineEdit("notegen-vision")
        form.addRow("Local AI endpoint", self.endpoint)
        form.addRow("Model alias", self.model)
        layout.addWidget(self.engine_settings)
        self.diagnostics_button = QPushButton("Check local engines & hardware")
        self.diagnostics_button.clicked.connect(self.run_doctor)
        layout.addWidget(self.diagnostics_button)
        self.diagnostics = QPlainTextEdit()
        self.diagnostics.setReadOnly(True)
        self.diagnostics.setPlaceholderText("Diagnostics appear here. No document data is sent.")
        layout.addWidget(self.diagnostics, 1)
        return page

    def settings(self) -> Settings:
        return Settings(
            mode=self.mode.currentData(),
            handwriting=self.handwriting.isChecked(),
            language=self.language.text().strip(),
            dpi=self.dpi.value(),
            context=self.context.toPlainText(),
            page_range=self.pages_input.text(),
            auto_rotate=self.auto_rotate.isChecked(),
            deskew=self.deskew.isChecked(),
            tesseract=self.tesseract.text().strip(),
            endpoint=self.endpoint.text().strip(),
            model=self.model.text().strip(),
        )

    def restore_preferences(self):
        if not self.persist_preferences:
            return
        for key, field in [
            ("language", self.language),
            ("endpoint", self.endpoint),
            ("model", self.model),
            ("tesseract", self.tesseract),
        ]:
            field.setText(str(self.preferences.value(key, field.text())))
        mode = self.preferences.value("mode", "balanced")
        self.mode.setCurrentIndex(max(0, self.mode.findData(mode)))
        self.dpi.setValue(int(self.preferences.value("dpi", 240)))

    def save_preferences(self):
        if self.persist_preferences:
            for key in ("language", "endpoint", "model", "tesseract"):
                self.preferences.setValue(key, getattr(self, key).text())
            self.preferences.setValue("mode", self.mode.currentData())
            self.preferences.setValue("dpi", self.dpi.value())
            self.preferences.setValue("output_dir", str(self.output_dir))

    def handwriting_mode(self, *_):
        if self.handwriting.isChecked() and self.mode.currentData() == "fast":
            self.mode.setCurrentIndex(0)

    def choose_files(self):
        paths, _ = QFileDialog.getOpenFileNames(
            self, "Choose scanned PDFs", "", "PDF files (*.pdf)"
        )
        self.add_files(paths)

    def add_files(self, paths):
        known = {q.source.resolve() for q in self.queue}
        for value in paths:
            path = Path(value)
            if path.is_file() and path.suffix.lower() == ".pdf" and path.resolve() not in known:
                known.add(path.resolve())
                self.queue.append(QueueItem(path))
                item = QListWidgetItem(path.name + "\nWaiting")
                item.setToolTip(str(path))
                self.file_list.addItem(item)
        self.empty_hint.setVisible(not self.queue)
        self.statusBar().showMessage(f"{len(self.queue)} document(s) in queue")

    def remove_selected(self):
        index = self.file_list.currentRow()
        if 0 <= index < len(self.queue) and not self.running_queue:
            self.queue.pop(index)
            self.file_list.takeItem(index)
            self.empty_hint.setVisible(not self.queue)

    def choose_output(self):
        path = QFileDialog.getExistingDirectory(self, "Choose output folder", str(self.output_dir))
        if path:
            self.output_dir = Path(path)
            self.output_label.setText(path)
            self.output_label.setToolTip(path)
            self.session_activity.path = self.output_dir / ".notegen" / "session.jsonl"
            self.save_preferences()

    def choose_tesseract(self):
        path, _ = QFileDialog.getOpenFileName(self, "Locate Tesseract executable")
        if path:
            self.tesseract.setText(path)

    def start_queue(self):
        if self.running_queue:
            return
        if not self.queue:
            self.statusBar().showMessage("Add at least one PDF first.")
            return
        if not self.review.save_block():
            return
        try:
            self.queue_settings = self.settings()
            self.output_dir.mkdir(parents=True, exist_ok=True)
            self.save_preferences()
        except Exception as exc:
            self.present_error(classify(exc))
            return
        for q in self.queue:
            if q.state in ("Failed", "Cancelled"):
                q.state = "Waiting"
        self.running_queue = True
        self.active_index = -1
        self.settings_form.setEnabled(False)
        self.engine_settings.setEnabled(False)
        self.choose_output_button.setEnabled(False)
        self.remove_button.setEnabled(False)
        self.open_project_button.setEnabled(False)
        self.start_button.setEnabled(False)
        self.next_job()

    def next_job(self):
        if self.close_requested or not self.running_queue:
            self.finish_queue()
            return
        index = next((i for i, q in enumerate(self.queue) if q.state == "Waiting"), -1)
        if index < 0:
            self.finish_queue()
            return
        self.active_index = index
        item = self.queue[index]
        item.state = "Converting"
        output = self.output_dir / (item.source.stem + ".docx")
        suffix = 2
        while output.exists():
            output = self.output_dir / f"{item.source.stem}-{suffix}.docx"
            suffix += 1
        item.output = output
        self.file_list.item(index).setText(item.source.name + "\nConverting")
        self.file_list.setCurrentRow(index)
        self.error_box.hide()
        self.started = time.monotonic()
        self.last_done = self.total = 0
        self.overall.setValue(0)
        self.page_progress.setValue(0)
        self.pause_button.setText("Pause")
        self.pause_button.setEnabled(True)
        self.cancel_button.setEnabled(True)
        self.worker = ConversionWorker(item.source, output, self.queue_settings, self)
        self.worker.event.connect(self.on_event)
        self.worker.result.connect(self.on_result)
        self.worker.failed.connect(self.on_failure)
        self.worker.finished.connect(self.worker_finished)
        self.worker.start()

    def on_event(self, event: Event):
        self.events.append(event)
        self.events = self.events[-6000:]
        try:
            self.session_activity.emit(
                event.stage,
                event.message,
                level=event.level,
                page=event.page,
                done=event.done,
                total=event.total,
                code=event.code,
                elapsed=event.elapsed,
            )
        except OSError:
            self.statusBar().showMessage(
                "Cannot write the session log. Check disk space and permissions."
            )
        if self.accept_log(event):
            self.log_view.appendPlainText(self.log_line(event))
        self.stage_label.setText(event.message)
        self.statusBar().showMessage(
            f"Page {event.page} · {event.stage}" if event.page else event.message
        )
        if event.total:
            self.last_done, self.total = event.done, event.total
            self.page_metric.value.setText(f"{event.done} / {event.total}")
            fraction = {
                "render": 5,
                "orientation": 12,
                "preprocess": 22,
                "ocr": 35,
                "vision": 60,
                "recovery": 65,
            }.get(event.stage, 0)
            if event.stage in ("page_done", "cached", "complete"):
                fraction = 0
                self.page_progress.setValue(100)
            elif fraction:
                self.page_progress.setValue(fraction)
            value = min(1000, round((event.done + fraction / 100) / event.total * 1000))
            self.overall.setValue(value)
            self.progress_text.setText(f"Document progress · {value / 10:.0f}%")

    @staticmethod
    def log_line(event):
        return f"{event.timestamp[11:19]}  {event.level.upper():7}  p{event.page:<4}  {event.stage:12}  {event.code} {event.message}"

    def accept_log(self, event):
        index = self.log_filter.currentIndex()
        return (
            index == 0
            or (index == 1 and event.level in ("warning", "error"))
            or (index == 2 and event.level == "error")
        )

    def refresh_log(self, *_):
        self.log_view.setPlainText(
            "\n".join(self.log_line(e) for e in self.events if self.accept_log(e))
        )

    def on_result(self, result):
        item = self.queue[self.active_index]
        item.state = "Completed"
        review_count = sum(
            p.needs_review(self.queue_settings.review_threshold) for p in result.pages
        )
        self.file_list.item(self.active_index).setText(
            item.source.name + f"\nCompleted · {review_count} page(s) to review"
        )
        self.last_output, self.last_project = result.output, result.project.root
        item.project = result.project.root
        self.review.load_project(result.project.root, result.pages)
        self.review_metric.value.setText(str(review_count))
        self.open_button.setEnabled(True)
        self.review_button.setEnabled(True)
        self.overall.setValue(1000)
        self.page_progress.setValue(100)
        self.stage_label.setText(
            "Word file ready" + (" · review flagged pages" if review_count else "")
        )

    def on_failure(self, error):
        item = self.queue[self.active_index]
        item.state = "Cancelled" if error.code == "NG900" else "Failed"
        self.file_list.item(self.active_index).setText(
            item.source.name + f"\n{item.state} · {error.code}"
        )
        self.on_event(
            Event(
                "error",
                error.diagnosis.title + ". " + error.diagnosis.action,
                level="error",
                code=error.code,
            )
        )
        self.present_error(error)
        if error.code in ("NG900", "NG300", "NG110", "NG200", "NG204", "NG400"):
            self.running_queue = False

    def worker_finished(self):
        self.worker.deleteLater()
        self.worker = None
        self.pause_button.setEnabled(False)
        self.cancel_button.setEnabled(False)
        QTimer.singleShot(0, self.next_job)

    def finish_queue(self):
        self.running_queue = False
        self.settings_form.setEnabled(True)
        self.engine_settings.setEnabled(True)
        self.choose_output_button.setEnabled(True)
        self.remove_button.setEnabled(True)
        self.open_project_button.setEnabled(True)
        self.start_button.setEnabled(True)
        self.pause_button.setEnabled(False)
        self.cancel_button.setEnabled(False)
        if self.close_requested:
            self.close()

    def toggle_pause(self):
        if self.worker and self.worker.isRunning():
            paused = self.worker.control.paused
            if paused.is_set():
                paused.clear()
                self.pause_button.setText("Pause")
            else:
                paused.set()
                self.pause_button.setText("Resume")
                self.statusBar().showMessage(
                    "Pause requested. Current OCR/AI operation will finish first."
                )

    def cancel_job(self):
        self.running_queue = False
        if self.worker:
            self.worker.control.cancel.set()
            self.cancel_button.setEnabled(False)
            self.statusBar().showMessage(
                "Cancelling. Completed pages are safe; waiting for the current operation to stop."
            )

    def present_error(self, error):
        self.current_error = error
        self.error_box.setPlainText(
            f"{error.code} · {error.diagnosis.title}\n"
            f"Why: {error.diagnosis.cause}\nFix: {error.diagnosis.action}"
        )
        self.error_box.show()
        self.stage_label.setText(error.diagnosis.title)

    def copy_error(self):
        if self.current_error:
            error = self.current_error
            QApplication.clipboard().setText(
                f"NoteGen {__version__} · {error}\n"
                f"{error.diagnosis.cause}\n{error.diagnosis.action}"
            )

    def tick(self):
        if self.worker and self.started:
            elapsed = int(time.monotonic() - self.started)
            self.time_metric.value.setText(f"{elapsed // 60:02}:{elapsed % 60:02}")
            if self.last_done and self.total > self.last_done:
                estimate = int(elapsed / self.last_done * (self.total - self.last_done))
                self.eta_label.setText(
                    f"Estimated time remaining: {estimate // 60:02}:{estimate % 60:02} · updates after each page"
                )
            elif self.total and self.last_done == self.total:
                self.eta_label.setText("All selected pages processed.")

    def poll_resources(self):
        if self.close_requested or (self.telemetry_task and self.telemetry_task.isRunning()):
            return
        self.telemetry_task = Task(snapshot, self)
        self.telemetry_task.result.connect(self.show_resources)
        self.telemetry_task.finished.connect(self.telemetry_done)
        self.telemetry_task.start()

    def show_resources(self, data: Resources):
        gpu = (
            f"{data.vram_used_mib / 1024:.1f} / {data.vram_total_mib / 1024:.0f} GB VRAM"
            if data.vram_total_mib
            else "GPU telemetry unavailable"
        )
        self.resource_label.setText(f"{data.ram_free_gib:.1f} GB RAM free · {gpu}")
        self.resource_label.setToolTip(data.gpu_name)

    def telemetry_done(self):
        self.telemetry_task.deleteLater()
        self.telemetry_task = None
        if self.close_requested:
            self.close()

    def run_doctor(self):
        if self.doctor_task and self.doctor_task.isRunning():
            return
        try:
            settings = self.settings()
        except Exception as exc:
            self.diagnostics.setPlainText(str(exc))
            return
        self.diagnostics_button.setEnabled(False)
        self.diagnostics.setPlainText("Checking local engines…")
        self.doctor_task = Task(lambda: doctor(settings), self)
        self.doctor_task.result.connect(
            lambda value: self.diagnostics.setPlainText(json.dumps(value, indent=2))
        )
        self.doctor_task.failed.connect(lambda error: self.diagnostics.setPlainText(str(error)))
        self.doctor_task.finished.connect(self.doctor_done)
        self.doctor_task.start()

    def doctor_done(self):
        self.doctor_task.deleteLater()
        self.doctor_task = None
        self.diagnostics_button.setEnabled(True)
        if self.close_requested:
            self.close()

    def open_output(self):
        if self.last_output:
            QDesktopServices.openUrl(QUrl.fromLocalFile(str(self.last_output.resolve())))

    def open_selected_job(self, *_):
        if self.running_queue:
            return
        index = self.file_list.currentRow()
        if 0 <= index < len(self.queue):
            item = self.queue[index]
            if item.state == "Completed" and item.project:
                try:
                    self.load_saved_project(item.project)
                    self.last_output = item.output
                    self.tabs.setCurrentIndex(1)
                except Exception as exc:
                    self.present_error(classify(exc))

    def open_project(self):
        root = QFileDialog.getExistingDirectory(
            self, "Open a folder containing project.json", str(self.output_dir / ".notegen")
        )
        if root:
            try:
                self.load_saved_project(Path(root))
                self.tabs.setCurrentIndex(1)
            except Exception as exc:
                self.present_error(classify(exc))

    def load_saved_project(self, root: Path):
        manifest = json.loads((root / "project.json").read_text(encoding="utf-8"))
        project = Project(
            root, manifest["input_sha256"], manifest["fingerprint"], manifest["pages"]
        )
        pages = [p for n in manifest["pages"] if (p := project.load(n)) is not None]
        self.review.load_project(root, pages)
        self.last_project = root
        self.update_review_count()

    def update_review_count(self):
        self.review_metric.value.setText(str(sum(p.needs_review() for p in self.review.pages)))

    def closeEvent(self, event):
        if not self.review.save_block():
            event.ignore()
            return
        busy = any(
            task and task.isRunning()
            for task in (self.worker, self.telemetry_task, self.doctor_task)
        )
        if busy:
            self.close_requested = True
            self.resource_timer.stop()
            self.cancel_job()
            self.statusBar().showMessage("Closing safely after the current operation stops…")
            event.ignore()
        else:
            self.save_preferences()
            event.accept()


def main():
    app = QApplication.instance() or QApplication(sys.argv)
    app.setApplicationName("NoteGen")
    app.setApplicationVersion(__version__)
    app.setStyle("Fusion")
    app.setStyleSheet(STYLE)
    window = MainWindow()
    window.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
