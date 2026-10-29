"""Run locally with your private corpus. Never uploads samples or guesses performance."""

import argparse
import json
import platform
import threading
import time
from pathlib import Path

from notegen import __version__
from notegen.events import Activity
from notegen.models import Settings
from notegen.pipeline import convert
from notegen.quality import transcription_metrics
from notegen.resources import snapshot
from notegen.storage import atomic_json


def main():
    p = argparse.ArgumentParser()
    p.add_argument("pdf", type=Path)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--report", type=Path, required=True)
    p.add_argument("--mode", choices=["fast", "balanced", "thorough"], default="thorough")
    p.add_argument("--expected", type=Path, help="Optional ground-truth text, max 5000 chars")
    p.add_argument("--context", default="")
    args = p.parse_args()
    if args.report.exists():
        p.error("Choose a new report path; existing benchmark reports are protected")
    stopped = threading.Event()
    samples = []

    def sample():
        while not stopped.is_set():
            samples.append(snapshot())
            stopped.wait(0.5)

    sampler = threading.Thread(target=sample, daemon=True)
    sampler.start()
    started = time.perf_counter()
    settings = Settings(mode=args.mode, context=args.context)
    try:
        result = convert(
            args.pdf,
            args.output,
            settings,
            Activity(callback=lambda e: print(e.stage, e.page, e.message, flush=True)),
        )
    finally:
        stopped.set()
        sampler.join(4)
    report = {
        "app_version": __version__,
        "platform": platform.platform(),
        "settings": settings.model_dump(),
        "seconds": time.perf_counter() - started,
        "pages": len(result.pages),
        "page_seconds": [p.seconds for p in result.pages],
        "review_pages": sum(p.needs_review() for p in result.pages),
        "peak_app_rss_gib": max((s.app_rss_gib for s in samples), default=None),
        "minimum_free_system_ram_gib": min((s.ram_free_gib for s in samples), default=None),
        "peak_gpu_used_mib_including_other_apps": max(
            (s.vram_used_mib for s in samples), default=None
        ),
        "note": "Sampled telemetry can miss brief peaks. App RSS excludes the external model server. Record runtime/model SHA256/driver separately.",
    }
    if args.expected:
        actual = "\n".join(
            b.text + " " + " ".join(c for r in b.rows for c in r)
            for page in result.pages
            for b in page.blocks
            if b.kind != "figure"
        )
        report["transcription"] = transcription_metrics(
            args.expected.read_text(encoding="utf-8"), actual
        )
    atomic_json(args.report, report)
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
