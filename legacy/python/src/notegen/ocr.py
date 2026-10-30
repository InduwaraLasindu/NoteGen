"""Tesseract subprocess adapter. No Python/CUDA/torch stack is loaded for OCR."""

import csv
import io
import os
import re
import shutil
import subprocess
import tempfile
import threading
import time
from dataclasses import dataclass
from pathlib import Path

import cv2
import numpy as np
from PIL import Image

from .errors import NoteGenError


@dataclass(frozen=True)
class Word:
    text: str
    x: int
    y: int
    w: int
    h: int
    confidence: float
    block: int = 0
    paragraph: int = 0
    line: int = 0


def find_tesseract(configured: str = "") -> str:
    candidates = [
        configured,
        shutil.which("tesseract"),
        str(
            Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Tesseract-OCR/tesseract.exe"
        ),
    ]
    for path in candidates:
        if path and Path(path).is_file():
            return str(Path(path).resolve())
    raise NoteGenError("NG110")


def run_process(
    args: list[str], timeout: float = 60, cancel: threading.Event | None = None
) -> tuple[int, str, str]:
    env = os.environ.copy()
    env["OMP_THREAD_LIMIT"] = "4"
    flags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
    # Disk-backed stdout/stderr: a noisy subprocess cannot deadlock a pipe or fill RAM.
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        try:
            process = subprocess.Popen(
                args, stdout=out, stderr=err, env=env, creationflags=flags, stdin=subprocess.DEVNULL
            )
        except OSError as exc:
            raise NoteGenError("NG110", type(exc).__name__) from exc
        start = time.monotonic()
        try:
            while process.poll() is None:
                if cancel and cancel.is_set():
                    raise NoteGenError("NG900")
                if time.monotonic() - start > timeout:
                    raise NoteGenError("NG111", "OCR timeout")
                try:
                    process.wait(timeout=0.1)
                except subprocess.TimeoutExpired:
                    pass
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        out.seek(0)
        err.seek(0)
        data = out.read(16_000_001)
        if len(data) > 16_000_000:
            raise NoteGenError("NG111", "Oversized OCR output")
        return (
            process.returncode,
            data.decode("utf-8", "replace"),
            err.read(8000).decode("utf-8", "replace"),
        )


def parse_tsv(data: str) -> list[Word]:
    words = []
    for r in csv.DictReader(io.StringIO(data), delimiter="\t", quoting=csv.QUOTE_NONE):
        try:
            text = r.get("text", "").strip()
            conf = float(r["conf"])
            if text and r["level"] == "5" and conf >= 0:
                words.append(
                    Word(
                        text,
                        int(r["left"]),
                        int(r["top"]),
                        int(r["width"]),
                        int(r["height"]),
                        min(1, conf / 100),
                        int(r["block_num"]),
                        int(r["par_num"]),
                        int(r["line_num"]),
                    )
                )
        except (ValueError, TypeError, KeyError):
            continue
    return words


class Tesseract:
    def __init__(self, executable: str = "", language: str = "eng"):
        if not re.fullmatch(r"[A-Za-z0-9_]+(\+[A-Za-z0-9_]+)*", language):
            raise NoteGenError("NG110", "Invalid language selector")
        self.executable = find_tesseract(executable)
        self.language = language

    def recognize(self, image: Image.Image, cancel: threading.Event | None = None) -> list[Word]:
        with tempfile.TemporaryDirectory(prefix="notegen-ocr-") as tmp:
            path = Path(tmp) / "scan.png"
            # Long grid lines can cause Tesseract's automatic page segmentation to
            # discard table contents. Remove them only in the OCR copy; retain the
            # original color page and grid for reconstruction and review.
            gray = np.array(image.convert("L"))
            ink = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY_INV + cv2.THRESH_OTSU)[1]
            h, w = gray.shape
            horizontal = cv2.morphologyEx(
                ink, cv2.MORPH_OPEN, np.ones((1, max(80, w // 8)), np.uint8)
            )
            vertical = cv2.morphologyEx(
                ink, cv2.MORPH_OPEN, np.ones((max(80, h // 12), 1), np.uint8)
            )
            mask = cv2.dilate(cv2.bitwise_or(horizontal, vertical), np.ones((3, 3), np.uint8))
            gray[mask > 0] = 255
            Image.fromarray(gray).save(path)
            code, output, _ = run_process(
                [self.executable, str(path), "stdout", "-l", self.language, "--psm", "3", "tsv"],
                cancel=cancel,
            )
            if code:
                raise NoteGenError("NG111", "Check language data and Tesseract installation")
            return parse_tsv(output)

    def orientation(self, image: Image.Image, cancel: threading.Event | None = None) -> int:
        small = image.copy()
        small.thumbnail((1800, 1800))
        with tempfile.TemporaryDirectory(prefix="notegen-osd-") as tmp:
            path = Path(tmp) / "orientation.png"
            small.save(path)
            try:
                code, text, _ = run_process(
                    [self.executable, str(path), "stdout", "--psm", "0"], timeout=15, cancel=cancel
                )
            except NoteGenError as exc:
                if exc.code == "NG900":
                    raise
                return 0
            match = re.search(r"Rotate:\s*(90|180|270)\b", text)
            confidence = re.search(r"Orientation confidence:\s*([\d.]+)", text)
            return (
                int(match[1])
                if not code and match and confidence and float(confidence[1]) >= 2
                else 0
            )
