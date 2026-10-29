import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pytest
from PIL import Image, ImageDraw, ImageFont


@pytest.fixture
def scanned_pdf(tmp_path):
    """Synthetic raster-only PDF. No hidden text layer or external document data."""
    image = Image.new("RGB", (1240, 1754), "white")
    d = ImageDraw.Draw(image)
    candidates = ["DejaVuSans.ttf", "C:/Windows/Fonts/arial.ttf"]
    font = None
    for candidate in candidates:
        try:
            font = ImageFont.truetype(candidate, 32)
            break
        except OSError:
            pass
    font = font or ImageFont.load_default(size=32)
    d.text((80, 90), "NoteGen local document recognition", fill="black", font=font)
    d.text((80, 155), "Revenue increased to 1250 in October.", fill="black", font=font)
    d.text((80, 220), "This document stays on your computer.", fill="black", font=font)
    for y in (400, 500, 600):
        d.line((80, y, 1100, y), fill="black", width=4)
    for x in (80, 600, 1100):
        d.line((x, 400, x, 600), fill="black", width=4)
    for pos, text in [
        ((110, 430), "Item"),
        ((630, 430), "Amount"),
        ((110, 530), "Sales"),
        ((630, 530), "1250"),
    ]:
        d.text(pos, text, fill="black", font=font)
    path = tmp_path / "scanned.pdf"
    image.save(path, "PDF", resolution=150)
    return path, image


@pytest.fixture
def project_root(tmp_path):
    root = tmp_path / "project"
    root.mkdir()
    return root
