"""Generate reproducible UI screenshots from a synthetic, local OCR sample."""

import os
import tempfile
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PIL import Image, ImageDraw, ImageFont
from PySide6.QtWidgets import QApplication

from notegen.app import MainWindow
from notegen.models import Settings
from notegen.pipeline import convert
from notegen.theme import STYLE


def main():
    root = Path(__file__).resolve().parents[1]
    target = root / "docs" / "images"
    target.mkdir(parents=True, exist_ok=True)
    app = QApplication([])
    app.setStyle("Fusion")
    app.setStyleSheet(STYLE)
    with tempfile.TemporaryDirectory(prefix="notegen-preview-") as tmp:
        tmp = Path(tmp)
        im = Image.new("RGB", (1240, 1754), "white")
        draw = ImageDraw.Draw(im)
        try:
            font = ImageFont.truetype("DejaVuSans.ttf", 32)
        except OSError:
            font = ImageFont.load_default(size=32)
        for y, text in [
            (90, "Accounting notes - October 2026"),
            (155, "Revenue increased to 1250 in October."),
            (220, "Keep the original scan for verification."),
        ]:
            draw.text((80, y), text, font=font, fill="black")
        for y in (400, 500, 600):
            draw.line((80, y, 1100, y), fill="black", width=4)
        for x in (80, 600, 1100):
            draw.line((x, 400, x, 600), fill="black", width=4)
        for xy, text in [
            ((110, 430), "Item"),
            ((630, 430), "Amount"),
            ((110, 530), "Sales"),
            ((630, 530), "1250"),
        ]:
            draw.text(xy, text, font=font, fill="black")
        source = tmp / "Accounting-notes.pdf"
        im.save(source, "PDF", resolution=150)
        events = []
        result = convert(
            source,
            tmp / "Accounting-notes.docx",
            Settings(mode="fast", dpi=150),
            activity=__import__("notegen.events", fromlist=["Activity"]).Activity(
                callback=events.append
            ),
        )
        window = MainWindow(tmp / "Converted-notes", persist=False, telemetry=False)
        window.mode.setCurrentIndex(window.mode.findData("fast"))
        window.add_files([str(source)])
        window.active_index = 0
        window.queue_settings = Settings(mode="fast")
        for event in events:
            window.on_event(event)
        window.on_result(result)
        window.output_label.setText("Documents / NoteGen")
        window.show()
        app.processEvents()
        window.grab().save(str(target / "workspace.png"))
        window.tabs.setCurrentIndex(1)
        window.review.block_list.setCurrentRow(
            next(i for i, b in enumerate(result.pages[0].blocks) if b.kind == "table")
        )
        window.review.splitter.setSizes([150, 500, 450])
        app.processEvents()
        window.grab().save(str(target / "review.png"))
        window.close()
    print(target)


if __name__ == "__main__":
    main()
