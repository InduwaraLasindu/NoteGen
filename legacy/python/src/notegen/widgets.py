from pathlib import Path

from PySide6.QtCore import Qt, Signal
from PySide6.QtGui import QColor, QPainter, QPen, QPixmap
from PySide6.QtWidgets import (
    QFrame,
    QGraphicsScene,
    QGraphicsView,
    QLabel,
    QListWidget,
    QVBoxLayout,
)

from .models import Box


def label(text: str, name: str = "", wrap: bool = False) -> QLabel:
    value = QLabel(text)
    value.setTextFormat(Qt.TextFormat.PlainText)
    if name:
        value.setObjectName(name)
    value.setWordWrap(wrap)
    return value


class Metric(QFrame):
    def __init__(self, title: str, value: str):
        super().__init__()
        self.setObjectName("card")
        layout = QVBoxLayout(self)
        layout.setContentsMargins(16, 12, 16, 12)
        layout.addWidget(label(title, "muted"))
        self.value = label(value, "metric")
        layout.addWidget(self.value)


class PdfList(QListWidget):
    files_dropped = Signal(list)

    def __init__(self):
        super().__init__()
        self.setAcceptDrops(True)
        self.setMinimumHeight(160)
        self.setToolTip("Drop one or more scanned PDF files here")

    def dragEnterEvent(self, event):
        if event.mimeData().hasUrls() and any(
            u.isLocalFile() and Path(u.toLocalFile()).suffix.lower() == ".pdf"
            for u in event.mimeData().urls()
        ):
            event.acceptProposedAction()
        else:
            event.ignore()

    def dragMoveEvent(self, event):
        event.acceptProposedAction()

    def dropEvent(self, event):
        paths = [
            u.toLocalFile()
            for u in event.mimeData().urls()
            if u.isLocalFile() and Path(u.toLocalFile()).suffix.lower() == ".pdf"
        ]
        self.files_dropped.emit(paths)
        event.acceptProposedAction()


class ScanView(QGraphicsView):
    def __init__(self):
        super().__init__()
        self.scene = QGraphicsScene(self)
        self.setScene(self.scene)
        self.setBackgroundBrush(QColor("#0a101c"))
        self.setRenderHint(QPainter.RenderHint.SmoothPixmapTransform)
        self.setDragMode(QGraphicsView.DragMode.ScrollHandDrag)
        self.setMinimumSize(280, 260)
        self.pixmap = None
        self.box_item = None
        self.auto_fit = True
        self.setToolTip("Ctrl + mouse wheel to zoom. Drag to pan. Fit page to reset.")

    def load(self, path: Path):
        self.scene.clear()
        self.box_item = None
        pixmap = QPixmap(str(path))
        self.pixmap = self.scene.addPixmap(pixmap)
        self.scene.setSceneRect(self.pixmap.boundingRect())
        self.auto_fit = True
        self.fit_page()

    def highlight(self, box: Box | None):
        if self.box_item:
            self.scene.removeItem(self.box_item)
            self.box_item = None
        if box and self.pixmap:
            size = self.pixmap.pixmap().size()
            left, top, right, bottom = box.pixels((size.width(), size.height()))
            pen = QPen(QColor("#11b69e"), 3)
            pen.setCosmetic(True)
            self.box_item = self.scene.addRect(left, top, right - left, bottom - top, pen)

    def fit_page(self):
        self.auto_fit = True
        if self.pixmap:
            self.fitInView(self.scene.sceneRect(), Qt.AspectRatioMode.KeepAspectRatio)

    def resizeEvent(self, event):
        super().resizeEvent(event)
        if self.auto_fit:
            self.fit_page()

    def wheelEvent(self, event):
        if event.modifiers() & Qt.KeyboardModifier.ControlModifier:
            self.auto_fit = False
            factor = 1.15 if event.angleDelta().y() > 0 else 1 / 1.15
            scale = self.transform().m11() * factor
            if 0.05 <= scale <= 8:
                self.scale(factor, factor)
            event.accept()
        else:
            super().wheelEvent(event)
