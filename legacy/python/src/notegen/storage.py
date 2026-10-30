"""Atomic, versioned, document-bound checkpoints."""

import hashlib
import json
import os
import tempfile
from pathlib import Path

from .errors import NoteGenError
from .models import PageResult


def atomic_json(path: Path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=".notegen-", suffix=".tmp")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(value, f, ensure_ascii=False, indent=2)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, path)
    finally:
        Path(tmp).unlink(missing_ok=True)


def file_hash(path: Path, cancel=None) -> str:
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        while chunk := f.read(1024 * 1024):
            if cancel and cancel.is_set():
                raise NoteGenError("NG900")
            h.update(chunk)
    return h.hexdigest()


class Project:
    def __init__(self, root: Path, input_hash: str, fingerprint: str, pages: list[int]):
        self.root = Path(root)
        self.root.mkdir(parents=True, exist_ok=True)
        self.manifest_path = self.root / "project.json"
        expected = {
            "schema": 1,
            "input_sha256": input_hash,
            "fingerprint": fingerprint,
            "pages": pages,
        }
        if self.manifest_path.exists():
            try:
                if json.loads(self.manifest_path.read_text(encoding="utf-8")) != expected:
                    raise NoteGenError("NG500")
            except (ValueError, OSError) as exc:
                raise NoteGenError("NG500", "Unreadable project manifest") from exc
        else:
            atomic_json(self.manifest_path, expected)

    def image_path(self, page: int) -> Path:
        return self.root / f"page-{page:05d}.png"

    def save(self, page: PageResult):
        atomic_json(self.root / f"page-{page.page:05d}.json", page.model_dump())

    def load(self, page: int) -> PageResult | None:
        path = self.root / f"page-{page:05d}.json"
        if not path.exists():
            return None
        try:
            result = PageResult.model_validate_json(path.read_text(encoding="utf-8"))
            if result.page != page or result.image != self.image_path(page).name:
                raise ValueError("Checkpoint path mismatch")
            if not self.image_path(page).is_file():
                return None
            return result
        except (ValueError, OSError):
            return None  # Recompute an incomplete/corrupt page, never trust its paths.


def safe_image(root: Path, name: str) -> Path:
    path = (root / name).resolve()
    if path.parent != root.resolve() or path.suffix.lower() != ".png":
        raise NoteGenError("NG500", "Unsafe checkpoint image path")
    return path
