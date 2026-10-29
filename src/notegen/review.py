"""Human review is separate from recognition; edits never silently rewrite Word files."""

from pathlib import Path

from PySide6.QtCore import Signal
from PySide6.QtWidgets import (
    QAbstractItemView,
    QCheckBox,
    QDoubleSpinBox,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QHeaderView,
    QListWidget,
    QMessageBox,
    QPlainTextEdit,
    QPushButton,
    QSplitter,
    QStackedWidget,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
    QWidget,
)

from .errors import classify
from .export import export_docx
from .models import Block, Box, PageResult
from .storage import atomic_json, safe_image
from .widgets import ScanView, label


class ReviewPanel(QWidget):
    changed = Signal()

    def __init__(self):
        super().__init__()
        self.pages: list[PageResult] = []
        self.root: Path | None = None
        self.page_index = -1
        self.block_index = -1
        self.dirty = False
        layout = QVBoxLayout(self)
        toolbar = QHBoxLayout()
        toolbar.addWidget(
            label("Compare with the scan, correct content, then export again.", "muted")
        )
        toolbar.addStretch()
        fit = QPushButton("Fit page")
        toolbar.addWidget(fit)
        self.export_button = QPushButton("Export corrected Word")
        self.export_button.setObjectName("primary")
        self.export_button.setEnabled(False)
        toolbar.addWidget(self.export_button)
        layout.addLayout(toolbar)
        self.splitter = QSplitter()
        layout.addWidget(self.splitter, 1)
        navigator = QWidget()
        nav = QVBoxLayout(navigator)
        nav.setContentsMargins(0, 0, 5, 0)
        nav.addWidget(label("PAGES", "eyebrow"))
        self.page_list = QListWidget()
        self.page_list.setMaximumWidth(230)
        nav.addWidget(self.page_list)
        nav.addWidget(label("PAGE REGIONS", "eyebrow"))
        self.block_list = QListWidget()
        self.block_list.setMaximumWidth(230)
        nav.addWidget(self.block_list, 2)
        self.splitter.addWidget(navigator)
        self.scan = ScanView()
        self.splitter.addWidget(self.scan)
        editor = QWidget()
        el = QVBoxLayout(editor)
        el.setContentsMargins(8, 0, 0, 0)
        self.details = label("Select a completed page to start reviewing.", "muted", True)
        el.addWidget(self.details)
        self.warnings = label("", "muted", True)
        el.addWidget(self.warnings)
        self.stack = QStackedWidget()
        self.text = QPlainTextEdit()
        self.text.setPlaceholderText("Editable recognized text")
        self.stack.addWidget(self.text)
        table_page = QWidget()
        tl = QVBoxLayout(table_page)
        tl.setContentsMargins(0, 0, 0, 0)
        self.table = QTableWidget()
        self.table.horizontalHeader().setSectionResizeMode(QHeaderView.ResizeMode.Stretch)
        self.table.setSelectionMode(QAbstractItemView.SelectionMode.ContiguousSelection)
        tl.addWidget(self.table)
        tb = QHBoxLayout()
        self.merge_button = QPushButton("Merge selection")
        self.unmerge_button = QPushButton("Split all merges")
        tb.addWidget(self.merge_button)
        tb.addWidget(self.unmerge_button)
        tl.addLayout(tb)
        self.stack.addWidget(table_page)
        figure = QWidget()
        fl = QFormLayout(figure)
        fl.setContentsMargins(0, 0, 0, 0)
        self.caption = QPlainTextEdit()
        self.caption.setMaximumHeight(60)
        self.observed = QPlainTextEdit()
        self.observed.setPlaceholderText("Visible labels, shapes, arrows and relationships")
        self.meaning = QPlainTextEdit()
        self.meaning.setPlaceholderText(
            "Meaning supported by the page; qualify uncertain inference"
        )
        fl.addRow("Caption", self.caption)
        fl.addRow("Observed", self.observed)
        fl.addRow("Interpretation", self.meaning)
        box_row = QHBoxLayout()
        self.box_fields = []
        for name in ("Left", "Top", "Right", "Bottom"):
            f = QDoubleSpinBox()
            f.setRange(0, 1)
            f.setSingleStep(0.01)
            f.setDecimals(3)
            f.setToolTip(name + " as a fraction of page size")
            box_row.addWidget(f)
            self.box_fields.append(f)
        fl.addRow("Crop L / T / R / B", box_row)
        self.stack.addWidget(figure)
        el.addWidget(self.stack, 1)
        self.uncertain = QCheckBox("This region is uncertain")
        el.addWidget(self.uncertain)
        self.save_button = QPushButton("Save region changes")
        el.addWidget(self.save_button)
        self.reviewed = QCheckBox("I have checked this page against the source")
        el.addWidget(self.reviewed)
        self.saved_status = label("Changes are saved in the local project.", "muted", True)
        el.addWidget(self.saved_status)
        self.splitter.addWidget(editor)
        self.splitter.setSizes([160, 560, 430])
        fit.clicked.connect(self.scan.fit_page)
        self.page_list.currentRowChanged.connect(self.select_page)
        self.block_list.currentRowChanged.connect(self.select_block)
        self.save_button.clicked.connect(self.save_block)
        self.reviewed.toggled.connect(self.mark_reviewed)
        self.merge_button.clicked.connect(self.merge_selection)
        self.unmerge_button.clicked.connect(self.split_merges)
        self.export_button.clicked.connect(self.export_corrected)
        for field in (self.text, self.caption, self.observed, self.meaning):
            field.textChanged.connect(self.mark_dirty)
        self.table.itemChanged.connect(self.mark_dirty)
        self.uncertain.toggled.connect(self.mark_dirty)
        for field in self.box_fields:
            field.valueChanged.connect(self.mark_dirty)

    def mark_dirty(self, *_):
        if self.block_index >= 0:
            self.dirty = True
            self.saved_status.setText("Unsaved region changes")

    def load_project(self, root: Path, pages: list[PageResult]):
        if not self.save_block():
            return
        self.root, self.pages = Path(root), [p.model_copy(deep=True) for p in pages]
        self.page_index = self.block_index = -1
        self.page_list.blockSignals(True)
        self.page_list.clear()
        for page in self.pages:
            self.page_list.addItem(self.page_title(page))
        self.page_list.blockSignals(False)
        self.export_button.setEnabled(bool(pages))
        if pages:
            self.page_list.setCurrentRow(0)

    @staticmethod
    def page_title(page):
        return f"Page {page.page}  ·  " + (
            "Reviewed" if page.reviewed else "Review needed" if page.needs_review() else "Ready"
        )

    def select_page(self, index: int):
        if not self.save_block():
            self.page_list.blockSignals(True)
            self.page_list.setCurrentRow(self.page_index)
            self.page_list.blockSignals(False)
            return
        self.page_index, self.block_index = index, -1
        self.block_list.blockSignals(True)
        self.block_list.clear()
        if index < 0 or index >= len(self.pages):
            self.block_list.blockSignals(False)
            return
        page = self.pages[index]
        try:
            self.scan.load(safe_image(self.root, page.image))
        except Exception as exc:
            self.show_error(exc)
        for block in page.blocks:
            detail = (
                f"{len(block.rows)} × {len(block.rows[0])}"
                if block.kind == "table"
                else (block.text or block.description).replace("\n", " ")[:35]
            )
            self.block_list.addItem(block.kind.title() + " · " + detail)
        self.block_list.blockSignals(False)
        self.warnings.setText("\n".join(page.warnings))
        self.reviewed.blockSignals(True)
        self.reviewed.setChecked(page.reviewed)
        self.reviewed.blockSignals(False)
        self.stack.setEnabled(bool(page.blocks))
        self.save_button.setEnabled(bool(page.blocks))
        if page.blocks:
            self.block_list.setCurrentRow(0)
        else:
            self.text.clear()
            self.details.setText("No text was recognized. The scan is preserved for manual review.")
        self.dirty = False

    def select_block(self, index: int):
        if not self.save_block():
            self.block_list.blockSignals(True)
            self.block_list.setCurrentRow(self.block_index)
            self.block_list.blockSignals(False)
            return
        self.block_index = -1
        if self.page_index < 0 or index < 0 or index >= len(self.pages[self.page_index].blocks):
            return
        block = self.pages[self.page_index].blocks[index]
        self.scan.highlight(block.box)
        rating = "not available" if block.confidence is None else f"{block.confidence:.0%}"
        self.details.setText(
            f"{block.kind.title()} · confidence: {rating}\n"
            "Confidence is an estimate. Compare with the source."
        )
        if block.kind == "table":
            self.stack.setCurrentIndex(1)
            self.table.clearSpans()
            self.table.setRowCount(len(block.rows))
            self.table.setColumnCount(len(block.rows[0]))
            for r, row in enumerate(block.rows):
                for c, text in enumerate(row):
                    self.table.setItem(r, c, QTableWidgetItem(text))
            for m in block.merges:
                if m.rowspan > 1 or m.colspan > 1:
                    self.table.setSpan(m.row, m.col, m.rowspan, m.colspan)
        elif block.kind == "figure":
            self.stack.setCurrentIndex(2)
            self.caption.setPlainText(block.text)
            self.observed.setPlainText(block.description)
            self.meaning.setPlainText(block.interpretation)
            for field, value in zip(self.box_fields, block.box.model_dump().values(), strict=True):
                field.setValue(value)
        else:
            self.stack.setCurrentIndex(0)
            self.text.setPlainText(block.text)
        self.uncertain.setChecked(block.uncertain)
        self.block_index = index
        self.dirty = False
        self.saved_status.setText("Changes are saved in the local project.")

    def persist(self):
        if self.root and self.page_index >= 0:
            page = self.pages[self.page_index]
            atomic_json(self.root / f"page-{page.page:05d}.json", page.model_dump())
            self.page_list.item(self.page_index).setText(self.page_title(page))
            self.changed.emit()

    def save_block(self, *_):
        if not self.dirty or self.page_index < 0 or self.block_index < 0:
            return True
        try:
            page = self.pages[self.page_index]
            data = page.blocks[self.block_index].model_dump()
            data["uncertain"] = self.uncertain.isChecked()
            if data["kind"] == "table":
                data["rows"] = [
                    [
                        self.table.item(r, c).text() if self.table.item(r, c) else ""
                        for c in range(self.table.columnCount())
                    ]
                    for r in range(self.table.rowCount())
                ]
            elif data["kind"] == "figure":
                data.update(
                    text=self.caption.toPlainText(),
                    description=self.observed.toPlainText(),
                    interpretation=self.meaning.toPlainText(),
                    box=Box(
                        **dict(
                            zip(
                                ("left", "top", "right", "bottom"),
                                (f.value() for f in self.box_fields),
                                strict=True,
                            )
                        )
                    ).model_dump(),
                )
            else:
                data["text"] = self.text.toPlainText()
            validated = Block.model_validate(data)
            page.blocks[self.block_index] = validated
            page.reviewed = False
            self.reviewed.blockSignals(True)
            self.reviewed.setChecked(False)
            self.reviewed.blockSignals(False)
            self.persist()
            self.scan.highlight(validated.box)
            self.dirty = False
            self.saved_status.setText("Saved. Export corrected Word when ready.")
            return True
        except Exception as exc:
            self.show_error(exc)
            return False

    def mark_reviewed(self, checked: bool):
        if not self.save_block() or self.page_index < 0:
            return
        self.pages[self.page_index].reviewed = checked
        self.reviewed.blockSignals(True)
        self.reviewed.setChecked(checked)
        self.reviewed.blockSignals(False)
        try:
            self.persist()
        except Exception as exc:
            self.show_error(exc)

    def merge_selection(self):
        if not self.save_block() or self.block_index < 0:
            return
        selected = self.table.selectedRanges()
        if len(selected) != 1:
            return
        area = selected[0]
        if area.rowCount() * area.columnCount() < 2:
            return
        try:
            page = self.pages[self.page_index]
            block = page.blocks[self.block_index]
            data = block.model_dump()
            merge = {
                "row": area.topRow(),
                "col": area.leftColumn(),
                "rowspan": area.rowCount(),
                "colspan": area.columnCount(),
            }
            data["merges"].append(merge)
            joined = []
            for r in range(area.topRow(), area.bottomRow() + 1):
                for c in range(area.leftColumn(), area.rightColumn() + 1):
                    if data["rows"][r][c]:
                        joined.append(data["rows"][r][c])
                    data["rows"][r][c] = ""
            data["rows"][area.topRow()][area.leftColumn()] = " ".join(joined)
            updated = Block.model_validate(data)
            page.blocks[self.block_index] = updated
            page.reviewed = False
            self.reviewed.blockSignals(True)
            self.reviewed.setChecked(False)
            self.reviewed.blockSignals(False)
            self.persist()
            self.select_block(self.block_index)
        except Exception as exc:
            self.show_error(exc)

    def split_merges(self):
        if not self.save_block() or self.block_index < 0:
            return
        page = self.pages[self.page_index]
        block = page.blocks[self.block_index]
        block.merges = []
        page.reviewed = False
        self.reviewed.blockSignals(True)
        self.reviewed.setChecked(False)
        self.reviewed.blockSignals(False)
        try:
            self.persist()
            self.select_block(self.block_index)
        except Exception as exc:
            self.show_error(exc)

    def export_to(self, path: Path):
        if not self.save_block():
            return None
        return export_docx(self.pages, self.root, path)

    def export_corrected(self):
        if not self.save_block():
            return
        path, _ = QFileDialog.getSaveFileName(
            self, "Export corrected Word", "Notes-reviewed.docx", "Word documents (*.docx)"
        )
        if path:
            try:
                result = self.export_to(Path(path))
                self.saved_status.setText(f"Saved: {result}")
            except Exception as exc:
                self.show_error(exc)

    def show_error(self, exc):
        error = classify(exc)
        QMessageBox.warning(self, error.code, str(exc) + "\n\n" + error.diagnosis.action)
