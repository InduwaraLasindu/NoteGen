"""Versioned page contracts shared by OCR, AI, review, cache and Word export."""

import hashlib
import ipaddress
import json
from typing import Literal
from urllib.parse import urlsplit

from pydantic import BaseModel, ConfigDict, Field, model_validator

from .errors import NoteGenError


def local_endpoint(value: str) -> str:
    try:
        u = urlsplit(value)
        if (
            u.scheme != "http"
            or not u.hostname
            or u.username
            or u.password
            or u.query
            or u.fragment
            or u.path not in ("", "/")
        ):
            raise ValueError
        if not ipaddress.ip_address(u.hostname).is_loopback:
            raise ValueError
        if u.port is None or not 1 <= u.port <= 65535:
            raise ValueError
    except ValueError as exc:
        raise NoteGenError("NG203") from exc
    return value.rstrip("/")


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", validate_assignment=True)


class Box(StrictModel):
    """Coordinates normalized to the processed page: [left, top, right, bottom]."""

    left: float = Field(ge=0, le=1)
    top: float = Field(ge=0, le=1)
    right: float = Field(ge=0, le=1)
    bottom: float = Field(ge=0, le=1)

    @model_validator(mode="after")
    def ordered(self):
        if self.right <= self.left or self.bottom <= self.top:
            raise ValueError("Box must have positive area")
        return self

    def pixels(self, size: tuple[int, int]) -> tuple[int, int, int, int]:
        w, h = size
        left, top = min(w - 1, int(self.left * w)), min(h - 1, int(self.top * h))
        return (
            left,
            top,
            max(left + 1, int(self.right * w)),
            max(top + 1, int(self.bottom * h)),
        )


class Merge(StrictModel):
    row: int = Field(ge=0)
    col: int = Field(ge=0)
    rowspan: int = Field(ge=1, le=100)
    colspan: int = Field(ge=1, le=40)


class Block(StrictModel):
    kind: Literal["text", "heading", "list", "table", "figure", "equation"]
    text: str = Field(default="", max_length=24000)
    box: Box | None = None
    confidence: float | None = Field(default=None, ge=0, le=1)
    rows: list[list[str]] = Field(default_factory=list, max_length=100)
    merges: list[Merge] = Field(default_factory=list, max_length=200)
    header_rows: int = Field(default=0, ge=0, le=10)
    description: str = Field(default="", max_length=6000)
    interpretation: str = Field(default="", max_length=6000)
    uncertain: bool = False

    @model_validator(mode="after")
    def valid_table_and_figure(self):
        if self.kind == "table":
            if not self.rows or not self.rows[0] or len(self.rows[0]) > 40:
                raise ValueError("Table must have 1–40 columns")
            cols = len(self.rows[0])
            if any(len(row) != cols for row in self.rows):
                raise ValueError(
                    "Table rows must have equal length; use empty strings for merged cells"
                )
            if self.header_rows > len(self.rows) or any(
                len(c) > 6000 for r in self.rows for c in r
            ):
                raise ValueError("Invalid table headers or oversized cells")
            occupied = set()
            for m in self.merges:
                if m.row + m.rowspan > len(self.rows) or m.col + m.colspan > cols:
                    raise ValueError("Merge outside table")
                for r in range(m.row, m.row + m.rowspan):
                    for c in range(m.col, m.col + m.colspan):
                        if (r, c) in occupied:
                            raise ValueError("Overlapping merges")
                        occupied.add((r, c))
        elif self.rows or self.merges:
            raise ValueError("Only tables may contain rows or merges")
        if self.kind == "figure" and self.box is None:
            raise ValueError("Figures require a source crop box")
        return self


class PageContent(StrictModel):
    blocks: list[Block] = Field(default_factory=list, max_length=300)
    warnings: list[str] = Field(default_factory=list, max_length=100)
    summary: str = Field(default="", max_length=2000)


class PageResult(PageContent):
    page: int = Field(ge=1)
    engine: str
    image: str
    seconds: float = Field(default=0, ge=0)
    reviewed: bool = False
    rotation: int = 0

    def needs_review(self, threshold: float = 0.85) -> bool:
        return not self.reviewed and (
            bool(self.warnings)
            or not self.blocks
            or any(
                b.uncertain
                or b.kind == "figure"
                or b.confidence is None
                or b.confidence < threshold
                for b in self.blocks
            )
        )


class Settings(StrictModel):
    mode: Literal["fast", "balanced", "thorough"] = "balanced"
    handwriting: bool = False
    language: str = Field(default="eng", pattern=r"^[A-Za-z0-9_]+(\+[A-Za-z0-9_]+)*$")
    dpi: int = Field(default=240, ge=100, le=400)
    max_pixels: int = Field(default=12_000_000, ge=1_000_000, le=20_000_000)
    ai_long_edge: int = Field(default=1920, ge=768, le=2560)
    max_tokens: int = Field(default=6144, ge=1024, le=8192)
    endpoint: str = "http://127.0.0.1:8080"
    model: str = "notegen-vision"
    tesseract: str = ""
    auto_rotate: bool = True
    deskew: bool = True
    review_threshold: float = Field(default=0.85, ge=0, le=1)
    context: str = Field(default="", max_length=2000)
    page_range: str = Field(default="", pattern=r"^[0-9,\- ]*$")
    request_timeout: int = Field(default=45, ge=5, le=300)

    @model_validator(mode="after")
    def validate_endpoint(self):
        local_endpoint(self.endpoint)
        return self

    def fingerprint(self, model_id: str = "") -> str:
        data = {"schema": 1, "settings": self.model_dump(), "model_id": model_id}
        return hashlib.sha256(json.dumps(data, sort_keys=True).encode()).hexdigest()


def select_pages(spec: str, total: int) -> list[int]:
    if not spec.strip():
        return list(range(1, total + 1))
    result = set()
    try:
        for part in spec.split(","):
            parts = part.strip().split("-")
            if len(parts) > 2:
                raise ValueError
            first = int(parts[0])
            last = int(parts[-1])
            if first < 1 or last > total or last < first:
                raise ValueError
            result.update(range(first, last + 1))
    except ValueError as exc:
        raise NoteGenError("NG100", "Invalid page range") from exc
    return sorted(result)
