"""Command line conversion and offline diagnostics."""

import argparse
import json
import sys
from pathlib import Path

from . import __version__
from .errors import NoteGenError, classify
from .events import Activity
from .export import export_docx
from .models import PageResult, Settings
from .ocr import find_tesseract
from .pipeline import Control, convert
from .resources import snapshot
from .vision import VisionClient


def doctor(settings: Settings) -> dict:
    result = {"version": __version__, "resources": snapshot().as_dict()}
    try:
        result["tesseract"] = find_tesseract(settings.tesseract)
    except NoteGenError as exc:
        result["tesseract"] = exc.code + ": " + exc.diagnosis.action
    vision = VisionClient(settings)
    try:
        result["vision_identity"] = vision.probe()
        result["vision_model"] = vision.model
    except NoteGenError as exc:
        result["vision"] = exc.code + ": " + exc.diagnosis.action
    finally:
        vision.close()
    return result


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(
        prog="notegen", description="Local scanned PDF to editable Word"
    )
    parser.add_argument("--version", action="version", version=__version__)
    parser.add_argument(
        "--debug", action="store_true", help="Print a local traceback for troubleshooting"
    )
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("convert")
    p.add_argument("pdf", type=Path)
    p.add_argument("-o", "--output", required=True, type=Path)
    p.add_argument("--mode", choices=["fast", "balanced", "thorough"], default="balanced")
    p.add_argument("--handwriting", action="store_true")
    p.add_argument("--language", default="eng")
    p.add_argument("--pages", default="")
    p.add_argument("--context", default="")
    p.add_argument("--dpi", type=int, default=240)
    p.add_argument("--tesseract", default="")
    p.add_argument("--endpoint", default="http://127.0.0.1:8080")
    p.add_argument("--model", default="notegen-vision")
    p.add_argument("--project", type=Path)
    p = sub.add_parser("doctor")
    p.add_argument("--endpoint", default="http://127.0.0.1:8080")
    p.add_argument("--tesseract", default="")
    p = sub.add_parser("export")
    p.add_argument("project", type=Path)
    p.add_argument("-o", "--output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "doctor":
            print(
                json.dumps(
                    doctor(Settings(endpoint=args.endpoint, tesseract=args.tesseract)), indent=2
                )
            )
        elif args.command == "export":
            manifest = json.loads((args.project / "project.json").read_text(encoding="utf-8"))
            pages = [
                PageResult.model_validate_json(
                    (args.project / f"page-{p:05d}.json").read_text(encoding="utf-8")
                )
                for p in manifest["pages"]
            ]
            print(export_docx(pages, args.project, args.output))
        else:
            settings = Settings(
                mode=args.mode,
                handwriting=args.handwriting,
                language=args.language,
                page_range=args.pages,
                context=args.context,
                dpi=args.dpi,
                tesseract=args.tesseract,
                endpoint=args.endpoint,
                model=args.model,
            )
            control = Control()
            try:
                result = convert(
                    args.pdf,
                    args.output,
                    settings,
                    control=control,
                    project_root=args.project,
                    activity=Activity(
                        callback=lambda e: print(
                            f"[{e.stage}] {e.done}/{e.total} p{e.page} {e.code} {e.message}",
                            flush=True,
                        )
                    ),
                )
            except KeyboardInterrupt:
                control.cancel.set()
                raise NoteGenError("NG900") from None
            print(f"Saved: {result.output}\nProject: {result.project.root}")
        return 0
    except Exception as exc:
        if args.debug:
            import traceback

            traceback.print_exc()
        error = classify(exc)
        print(f"{error}\n{error.diagnosis.cause}\nFix: {error.diagnosis.action}", file=sys.stderr)
        return 130 if error.code == "NG900" else 1


if __name__ == "__main__":
    raise SystemExit(main())
