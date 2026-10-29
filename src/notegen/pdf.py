"""PDFium stays on one worker; bounded rendering, no PDF actions or JavaScript."""

import math
from pathlib import Path

import pypdfium2 as pdfium

from .errors import NoteGenError


class PdfSource:
    def __init__(self, path: Path, max_pages: int = 3000):
        self.path = Path(path)
        self.doc = None
        try:
            if self.path.suffix.lower() != ".pdf" or self.path.stat().st_size > 2_000_000_000:
                raise NoteGenError("NG101", "Expected PDF smaller than 2 GB")
            self.doc = pdfium.PdfDocument(str(self.path))
            self.count = len(self.doc)
            if not 0 < self.count <= max_pages:
                raise NoteGenError("NG101", "Invalid or excessive page count")
        except Exception as exc:
            self.close()
            if isinstance(exc, NoteGenError):
                raise
            raise NoteGenError("NG100", type(exc).__name__) from exc

    def render(self, page: int, dpi: int = 240, max_pixels: int = 12_000_000):
        if not 1 <= page <= self.count:
            raise NoteGenError("NG100", "Page outside PDF")
        try:
            handle = self.doc[page - 1]
            try:
                w, h = handle.get_size()
                if not all(math.isfinite(x) and 0 < x < 1_000_000 for x in (w, h)):
                    raise NoteGenError("NG101")
                scale = min(dpi / 72, math.sqrt(max_pixels / (w * h)), 10000 / max(w, h))
                # Ceiling padding in PDFium can add one row/column; leave a margin.
                scale *= 0.999
                bitmap = handle.render(scale=scale)
                try:
                    return bitmap.to_pil().convert("RGB").copy()
                finally:
                    bitmap.close()
            finally:
                handle.close()
        except NoteGenError:
            raise
        except Exception as exc:
            raise NoteGenError("NG100", f"Cannot render page {page}") from exc

    def close(self):
        if self.doc:
            self.doc.close()
            self.doc = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
