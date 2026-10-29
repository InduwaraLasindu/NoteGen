"""Structured activity without recognized document text in the log."""

import json
import threading
from collections.abc import Callable
from dataclasses import asdict, dataclass
from datetime import UTC, datetime
from pathlib import Path


@dataclass(frozen=True)
class Event:
    stage: str
    message: str
    level: str = "info"
    page: int = 0
    done: int = 0
    total: int = 0
    code: str = ""
    elapsed: float = 0
    timestamp: str = ""


class Activity:
    def __init__(self, path: Path | None = None, callback: Callable[[Event], None] | None = None):
        self.path, self.callback = path, callback
        self.lock = threading.Lock()

    def emit(self, stage: str, message: str, **kwargs):
        event = Event(stage, message, timestamp=datetime.now(UTC).isoformat(), **kwargs)
        if self.path:
            with self.lock:
                self.path.parent.mkdir(parents=True, exist_ok=True)
                with self.path.open("a", encoding="utf-8") as f:
                    f.write(json.dumps(asdict(event), ensure_ascii=False) + "\n")
        if self.callback:
            self.callback(event)
        return event
