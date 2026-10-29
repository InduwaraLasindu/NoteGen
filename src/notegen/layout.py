"""Conservative ruled-table and figure candidates for CPU OCR and AI routing."""

from collections import defaultdict

import cv2
import numpy as np
from PIL import Image

from .imaging import box_from_pixels
from .models import Block, PageContent
from .ocr import Word


def _centers(values: np.ndarray) -> list[int]:
    groups = []
    for index in np.flatnonzero(values):
        if not groups or index - groups[-1][-1] > 3:
            groups.append([])
        groups[-1].append(int(index))
    return [int(np.mean(g)) for g in groups]


def ruled_tables(image: Image.Image, words: list[Word]) -> list[Block]:
    gray = np.array(image.convert("L"))
    binary = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY_INV + cv2.THRESH_OTSU)[1]
    h, w = gray.shape
    horizontal = cv2.morphologyEx(
        binary, cv2.MORPH_OPEN, cv2.getStructuringElement(cv2.MORPH_RECT, (max(30, w // 25), 1))
    )
    vertical = cv2.morphologyEx(
        binary, cv2.MORPH_OPEN, cv2.getStructuringElement(cv2.MORPH_RECT, (1, max(30, h // 30)))
    )
    grid = cv2.bitwise_or(horizontal, vertical)
    contours, _ = cv2.findContours(grid, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    blocks = []
    for contour in contours:
        x, y, tw, th = cv2.boundingRect(contour)
        if tw < w * 0.15 or th < 50:
            continue
        xs = _centers(np.sum(vertical[y : y + th, x : x + tw] > 0, axis=0) > th * 0.5)
        ys = _centers(np.sum(horizontal[y : y + th, x : x + tw] > 0, axis=1) > tw * 0.5)
        if not (3 <= len(xs) <= 41 and 3 <= len(ys) <= 101):
            continue
        rows = [["" for _ in range(len(xs) - 1)] for _ in range(len(ys) - 1)]
        confidence = []
        for word in sorted(words, key=lambda v: (v.y // max(10, v.h), v.x)):
            cx, cy = word.x + word.w / 2 - x, word.y + word.h / 2 - y
            col, row = np.searchsorted(xs, cx) - 1, np.searchsorted(ys, cy) - 1
            if 0 <= col < len(xs) - 1 and 0 <= row < len(ys) - 1:
                rows[row][col] += (" " if rows[row][col] else "") + word.text
                confidence.append(word.confidence)
        blocks.append(
            Block(
                kind="table",
                rows=rows,
                box=box_from_pixels(x, y, tw, th, image.size),
                confidence=float(np.mean(confidence)) if confidence else 0,
                uncertain=not confidence,
            )
        )
    return sorted(blocks, key=lambda b: b.box.top)


def _inside(word: Word, block: Block, size: tuple[int, int]):
    if block.box is None:
        return False
    x, y = (word.x + word.w / 2) / size[0], (word.y + word.h / 2) / size[1]
    return block.box.left <= x <= block.box.right and block.box.top <= y <= block.box.bottom


def visual_candidates(image: Image.Image, words: list[Word], tables: list[Block]) -> list[Block]:
    gray = np.array(image.convert("L"))
    binary = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY_INV + cv2.THRESH_OTSU)[1]
    h, w = gray.shape
    for word in words:
        cv2.rectangle(
            binary,
            (max(0, word.x - 3), max(0, word.y - 3)),
            (word.x + word.w + 3, word.y + word.h + 3),
            0,
            -1,
        )
    for table in tables:
        left, top, right, bottom = table.box.pixels(image.size)
        cv2.rectangle(binary, (left - 4, top - 4), (right + 4, bottom + 4), 0, -1)
    joined = cv2.morphologyEx(binary, cv2.MORPH_CLOSE, np.ones((15, 15), np.uint8))
    contours, _ = cv2.findContours(joined, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    figures = []
    for c in contours:
        x, y, fw, fh = cv2.boundingRect(c)
        if fw > w * 0.08 and fh > h * 0.04 and fw * fh > w * h * 0.005:
            figures.append(
                Block(
                    kind="figure",
                    box=box_from_pixels(x, y, fw, fh, image.size),
                    description="Possible diagram or unrecognized region. Review the source.",
                    uncertain=True,
                )
            )
    return figures[:30]


def extract_layout(image: Image.Image, words: list[Word]) -> PageContent:
    tables = ruled_tables(image, words)
    paragraphs = defaultdict(list)
    for word in words:
        if not any(_inside(word, t, image.size) for t in tables):
            paragraphs[(word.block, word.paragraph)].append(word)
    blocks = []
    for group in paragraphs.values():
        lines = defaultdict(list)
        for word in group:
            lines[word.line].append(word.text)
        text = "\n".join(" ".join(row) for row in lines.values())
        x, y = min(v.x for v in group), min(v.y for v in group)
        r, b = max(v.x + v.w for v in group), max(v.y + v.h for v in group)
        blocks.append(
            Block(
                kind="text",
                text=text,
                confidence=float(np.mean([v.confidence for v in group])),
                box=box_from_pixels(x, y, r - x, b - y, image.size),
            )
        )
    blocks.extend(tables)
    blocks.extend(visual_candidates(image, words, tables))
    blocks.sort(key=lambda b: (round(b.box.top, 2), b.box.left))
    warnings = []
    if not words:
        warnings.append("No text recognized. This may be a blank, handwritten or unreadable page.")
    return PageContent(
        blocks=blocks[:300], warnings=warnings, summary=" ".join(w.text for w in words)[:1500]
    )
