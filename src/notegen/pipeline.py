"""Sequential, checkpointed conversion; the GUI thread never touches PDFium."""

import re
import threading
import time
from dataclasses import dataclass
from pathlib import Path

from .errors import NoteGenError
from .events import Activity
from .export import export_docx
from .imaging import prepare
from .layout import extract_layout
from .models import PageContent, PageResult, Settings, select_pages
from .ocr import Tesseract
from .resources import guard, snapshot
from .storage import Project, file_hash
from .vision import VisionClient


class Control:
    def __init__(self):
        self.cancel = threading.Event()
        self.paused = threading.Event()

    def checkpoint(self, activity: Activity):
        if self.paused.is_set() and not self.cancel.is_set():
            activity.emit("paused", "Paused. Completed pages are safe.")
            while self.paused.is_set() and not self.cancel.wait(0.1):
                pass
            activity.emit("resume", "Continuing conversion")
        if self.cancel.is_set():
            raise NoteGenError("NG900")


@dataclass
class Conversion:
    output: Path
    project: Project
    pages: list[PageResult]


def wants_vision(settings: Settings, content: PageContent) -> bool:
    if settings.mode == "fast":
        return False
    return (
        settings.mode == "thorough"
        or settings.handwriting
        or not content.blocks
        or any(
            b.kind in ("figure", "table")
            or b.confidence is None
            or b.confidence < settings.review_threshold
            for b in content.blocks
        )
    )


def check_disagreement(ocr: PageContent, ai: PageContent):
    def text(page):
        return " ".join(
            b.text + " " + " ".join(c for row in b.rows for c in row) for b in page.blocks
        )

    original = set(re.findall(r"\b\d+(?:[.,]\d+)*\b", text(ocr)))
    recognized = set(re.findall(r"\b\d+(?:[.,]\d+)*\b", text(ai)))
    if original - recognized:
        ai.warnings.append("OCR and AI disagree on one or more numbers. Verify against the scan.")
    if len(text(ocr).strip()) > 100 and len(text(ai).strip()) < len(text(ocr).strip()) * 0.35:
        ai.warnings.append("AI returned much less text than OCR. Check for omitted regions.")


def convert(
    source: Path,
    output: Path,
    settings: Settings,
    activity: Activity | None = None,
    control: Control | None = None,
    project_root: Path | None = None,
    vision_factory=VisionClient,
    resource_reader=snapshot,
) -> Conversion:
    from .pdf import PdfSource

    source, output = Path(source), Path(output)
    if source.resolve() == output.resolve():
        raise NoteGenError("NG400", "Input and output must differ")
    if output.exists():
        raise NoteGenError("NG401")
    activity, control = activity or Activity(), control or Control()
    activity.emit("prepare", "Validating PDF and hashing input")
    control.checkpoint(activity)
    try:
        digest = file_hash(source, control.cancel)
    except OSError as exc:
        raise NoteGenError("NG100", type(exc).__name__) from exc
    vision = None
    ai_error = None
    model_identity = "cpu-only"
    try:
        if settings.mode != "fast":
            candidate = vision_factory(settings)
            try:
                model_identity = candidate.probe()
                vision = candidate
                activity.emit("model", "Local vision model is ready")
            except NoteGenError as exc:
                candidate.close()
                if settings.mode == "thorough" or settings.handwriting:
                    raise
                ai_error = exc
                model_identity = "vision-unavailable"
                activity.emit(
                    "model",
                    "Local AI unavailable; preserving CPU OCR for review",
                    level="warning",
                    code=exc.code,
                )
        try:
            ocr = Tesseract(settings.tesseract, settings.language)
        except NoteGenError:
            if not vision:
                raise
            ocr = None
            activity.emit(
                "ocr",
                "Tesseract unavailable; using local vision for all pages",
                level="warning",
                code="NG110",
            )
        with PdfSource(source) as pdf:
            selected = select_pages(settings.page_range, pdf.count)
            fingerprint = settings.fingerprint(model_identity)
            root = project_root or output.parent / ".notegen" / (
                source.stem + "-" + digest[:10] + "-" + fingerprint[:10]
            )
            project = Project(root, digest, fingerprint, selected)
            # Keep log/checkpoints together even when the caller supplied a UI-only logger.
            if activity.path is None:
                activity.path = project.root / "activity.jsonl"
            activity.emit("start", "Conversion started", total=len(selected))
            results = []
            previous = ""
            for done, number in enumerate(selected):
                control.checkpoint(activity)
                guard(resource_reader())
                cached = project.load(number)
                if cached:
                    results.append(cached)
                    previous = cached.summary
                    activity.emit(
                        "cached",
                        "Loaded completed page checkpoint",
                        page=number,
                        done=done + 1,
                        total=len(selected),
                    )
                    continue
                started = time.monotonic()

                def event(stage, message, _page=number, _done=done, _started=started, **kwargs):
                    activity.emit(
                        stage,
                        message,
                        page=_page,
                        done=_done,
                        total=len(selected),
                        elapsed=time.monotonic() - _started,
                        **kwargs,
                    )

                event("render", "Rendering page within image memory limit")
                image = pdf.render(number, settings.dpi, settings.max_pixels)
                rotation = 0
                if ocr and settings.auto_rotate:
                    event("orientation", "Checking page orientation")
                    rotation = ocr.orientation(image, control.cancel)
                    if rotation:
                        image = image.rotate(-rotation, expand=True)
                        event("orientation", f"Corrected {rotation}° page rotation")
                control.checkpoint(activity)
                event("preprocess", "Preparing scan and checking image quality")
                prepared = prepare(image, settings.deskew)
                image.close()
                image = prepared.image
                path = project.image_path(number)
                temp = path.with_suffix(".tmp.png")
                image.save(temp)
                temp.replace(path)
                event("ocr", "Recognizing printed text and rebuilding ruled tables")
                try:
                    words = ocr.recognize(prepared.ocr, control.cancel) if ocr else []
                    content = extract_layout(image, words)
                except NoteGenError as exc:
                    if exc.code == "NG900" or not vision:
                        raise
                    content = PageContent(warnings=["CPU OCR failed; local AI used for this page."])
                    event("ocr", "CPU OCR failed; trying local AI", level="warning", code=exc.code)
                prepared.ocr.close()
                warnings = prepared.warnings + content.warnings
                engine = "tesseract"
                if settings.handwriting and settings.mode == "fast":
                    warnings.append("Fast mode is not suitable for handwriting. Select an AI mode.")
                needs_ai = ocr is None or wants_vision(settings, content)
                if needs_ai and vision:
                    control.checkpoint(activity)
                    guard(resource_reader())
                    event("vision", "Local AI is reading text, tables and diagrams")
                    evidence = "\n".join(b.text + " " + str(b.rows) for b in content.blocks)[:10000]
                    try:
                        recognized = vision.recognize(
                            image,
                            evidence,
                            settings.context + "\nPrevious selected page: " + previous,
                            control.cancel,
                            on_tokens=lambda n: event(
                                "vision", f"Local AI generated {n} stream fragments"
                            ),
                            on_retry=lambda exc: event(
                                "recovery",
                                "Retrying AI at reduced image size",
                                level="warning",
                                code=exc.code,
                            ),
                        )
                        check_disagreement(content, recognized)
                        if any(b.kind == "figure" for b in recognized.blocks):
                            recognized.warnings.append(
                                "Diagram meanings are AI interpretations; verify the source."
                            )
                        content = recognized
                        warnings.extend(recognized.warnings)
                        engine = "local-vision"
                    except NoteGenError as exc:
                        if exc.code == "NG900":
                            raise
                        warnings.append(
                            f"{exc.code}: AI failed; CPU OCR preserved. Review all content."
                        )
                        event(
                            "recovery",
                            "AI failed; retaining OCR and source image for review",
                            level="warning",
                            code=exc.code,
                        )
                elif ai_error:
                    warnings.append(f"{ai_error.code}: Local AI was unavailable; CPU OCR only.")
                elif needs_ai and not vision:
                    warnings.append(
                        "CPU OCR only. Diagram meanings and handwriting need local AI or manual review."
                    )
                if not content.blocks:
                    warnings.append(
                        "No content recognized. The original page is preserved in this project."
                    )
                result = PageResult(
                    page=number,
                    engine=engine,
                    image=path.name,
                    blocks=content.blocks,
                    warnings=list(dict.fromkeys(warnings))[:100],
                    summary=content.summary,
                    rotation=rotation,
                    seconds=time.monotonic() - started,
                )
                project.save(result)
                image.close()
                results.append(result)
                previous = result.summary
                activity.emit(
                    "page_done",
                    "Page checkpoint saved"
                    + (
                        " — review needed" if result.needs_review(settings.review_threshold) else ""
                    ),
                    page=number,
                    done=done + 1,
                    total=len(selected),
                    elapsed=result.seconds,
                )
            control.checkpoint(activity)
            activity.emit(
                "export", "Writing editable Word document", done=len(selected), total=len(selected)
            )
            export_docx(results, project.root, output, source.stem, control.cancel)
            activity.emit(
                "complete", "Word document saved", done=len(selected), total=len(selected)
            )
            return Conversion(output, project, results)
    finally:
        if vision:
            vision.close()
