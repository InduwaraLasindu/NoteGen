import threading

import pytest
from docx import Document
from PIL import Image

import notegen.pipeline as pipeline
from notegen.errors import NoteGenError
from notegen.events import Activity
from notegen.models import Block, PageContent, Settings
from notegen.ocr import Word
from notegen.pipeline import Control, check_disagreement, convert, wants_vision
from notegen.resources import Resources, guard


class FakeOCR:
    calls = 0

    def __init__(self, *args):
        pass

    def orientation(self, image, cancel):
        return 0

    def recognize(self, image, cancel):
        FakeOCR.calls += 1
        return [
            Word("Revenue", 100, 100, 100, 40, 0.95, 1, 1, 1),
            Word("1250", 220, 100, 80, 40, 0.95, 1, 1, 1),
        ]


class FakeVision:
    def __init__(self, settings):
        self.closed = False

    def probe(self):
        return "test-model-identity"

    def close(self):
        self.closed = True

    def recognize(self, image, evidence, context, cancel, **kwargs):
        return PageContent(
            blocks=[Block(kind="text", text="Revenue 1250", confidence=0.95)],
            summary="Revenue 1250",
        )


def no_gpu():
    return Resources(10, 0.3)


@pytest.fixture
def fake_ocr(monkeypatch):
    FakeOCR.calls = 0
    monkeypatch.setattr(pipeline, "Tesseract", FakeOCR)


def test_routing_and_disagreement():
    content = PageContent(blocks=[Block(kind="text", text="1250", confidence=0.99)])
    assert not wants_vision(Settings(), content)
    assert wants_vision(Settings(handwriting=True), content)
    assert wants_vision(Settings(mode="thorough"), content)
    assert not wants_vision(Settings(mode="fast"), content)
    blurry = content.model_copy(deep=True)
    blurry.warnings.append("Possible blur")
    assert wants_vision(Settings(), blurry)
    ai = PageContent(blocks=[Block(kind="text", text="1200")])
    check_disagreement(content, ai)
    assert any("numbers" in w for w in ai.warnings)


def test_pipeline_success_and_cached_resume(scanned_pdf, tmp_path, fake_ocr):
    path, _ = scanned_pdf
    activity = []
    project = tmp_path / "project"
    first = convert(
        path,
        tmp_path / "a.docx",
        Settings(mode="fast", dpi=150),
        Activity(callback=activity.append),
        project_root=project,
        resource_reader=no_gpu,
    )
    assert first.output.exists() and first.project.load(1)
    assert FakeOCR.calls == 1
    assert "Revenue 1250" in " ".join(p.text for p in Document(first.output).paragraphs)
    convert(
        path,
        tmp_path / "b.docx",
        Settings(mode="fast", dpi=150),
        Activity(callback=activity.append),
        project_root=project,
        resource_reader=no_gpu,
    )
    assert FakeOCR.calls == 1
    assert any(e.stage == "cached" for e in activity)
    with pytest.raises(NoteGenError, match="NG500"):
        convert(
            path,
            tmp_path / "c.docx",
            Settings(mode="fast", dpi=200),
            project_root=project,
            resource_reader=no_gpu,
        )


def test_cancellation_after_checkpoint_can_resume(scanned_pdf, tmp_path, fake_ocr):
    path, _ = scanned_pdf
    control = Control()

    def event(e):
        if e.stage == "page_done":
            control.cancel.set()

    with pytest.raises(NoteGenError, match="NG900"):
        convert(
            path,
            tmp_path / "a.docx",
            Settings(mode="fast", dpi=150),
            Activity(callback=event),
            control,
            tmp_path / "project",
            resource_reader=no_gpu,
        )
    assert not (tmp_path / "a.docx").exists()
    result = convert(
        path,
        tmp_path / "a.docx",
        Settings(mode="fast", dpi=150),
        project_root=tmp_path / "project",
        resource_reader=no_gpu,
    )
    assert result.output.exists() and FakeOCR.calls == 1


def test_pause_is_cooperative_and_cancel_releases_it():
    control = Control()
    control.paused.set()
    reached = threading.Event()
    errors = []

    def work():
        try:
            control.checkpoint(Activity(callback=lambda _: reached.set()))
        except NoteGenError as exc:
            errors.append(exc.code)

    thread = threading.Thread(target=work)
    thread.start()
    assert reached.wait(2)
    control.cancel.set()
    thread.join(2)
    assert not thread.is_alive() and errors == ["NG900"]


def test_thorough_ai_and_fallback(scanned_pdf, tmp_path, fake_ocr):
    path, _ = scanned_pdf
    result = convert(
        path,
        tmp_path / "ai.docx",
        Settings(mode="thorough", dpi=150),
        vision_factory=FakeVision,
        resource_reader=no_gpu,
    )
    assert result.pages[0].engine == "local-vision"

    class Failing(FakeVision):
        def recognize(self, *args, **kwargs):
            raise NoteGenError("NG202")

    result = convert(
        path,
        tmp_path / "fallback.docx",
        Settings(mode="thorough", dpi=150),
        project_root=tmp_path / "fallback",
        vision_factory=Failing,
        resource_reader=no_gpu,
    )
    assert result.pages[0].engine == "tesseract"
    assert result.pages[0].needs_review()
    assert "NG202" in " ".join(result.pages[0].warnings)


def test_missing_ai_is_explicit_and_required_for_handwriting(scanned_pdf, tmp_path, fake_ocr):
    class Missing(FakeVision):
        def probe(self):
            raise NoteGenError("NG200")

    path, _ = scanned_pdf
    result = convert(
        path,
        tmp_path / "balanced.docx",
        Settings(dpi=150),
        vision_factory=Missing,
        resource_reader=no_gpu,
    )
    assert result.pages[0].needs_review()
    for settings in [Settings(mode="thorough"), Settings(handwriting=True)]:
        with pytest.raises(NoteGenError, match="NG200"):
            convert(
                path,
                tmp_path / "required.docx",
                settings,
                vision_factory=Missing,
                resource_reader=no_gpu,
            )


def test_vision_only_operation_when_tesseract_missing(scanned_pdf, tmp_path, monkeypatch):
    def missing(*_):
        raise NoteGenError("NG110")

    monkeypatch.setattr(pipeline, "Tesseract", missing)
    path, _ = scanned_pdf
    result = convert(
        path,
        tmp_path / "vision-only.docx",
        Settings(mode="thorough", dpi=150),
        vision_factory=FakeVision,
        resource_reader=no_gpu,
    )
    assert result.pages[0].engine == "local-vision"


def test_memory_guard_preserves_existing_work(scanned_pdf, tmp_path, fake_ocr):
    for resources in [
        Resources(0.2, 0.3),
        Resources(10, 4.1),
        Resources(10, 0.3, "RTX", 15000, 16384),
    ]:
        with pytest.raises(NoteGenError, match="NG300"):
            guard(resources)
    path, _ = scanned_pdf
    with pytest.raises(NoteGenError, match="NG300"):
        convert(
            path,
            tmp_path / "memory.docx",
            Settings(mode="fast"),
            resource_reader=lambda: Resources(0.1, 0.3),
        )
    assert not (tmp_path / "memory.docx").exists()


def test_multiple_pages_and_selected_range(tmp_path, fake_ocr):
    image = Image.new("RGB", (400, 600), "white")
    path = tmp_path / "many.pdf"
    image.save(path, "PDF", save_all=True, append_images=[image, image])
    result = convert(
        path,
        tmp_path / "subset.docx",
        Settings(mode="fast", dpi=100, page_range="1,3"),
        resource_reader=no_gpu,
    )
    assert [p.page for p in result.pages] == [1, 3]
    assert len(Document(result.output).sections) == 1
    assert "Source page 3" in " ".join(p.text for p in Document(result.output).paragraphs)


def test_unrecognized_page_is_preserved_in_word(tmp_path, monkeypatch):
    class EmptyOCR(FakeOCR):
        def recognize(self, *args):
            return []

    monkeypatch.setattr(pipeline, "Tesseract", EmptyOCR)
    source = tmp_path / "blank.pdf"
    Image.new("RGB", (400, 600), "white").save(source, "PDF")
    result = convert(
        source, tmp_path / "preserved.docx", Settings(mode="fast", dpi=100), resource_reader=no_gpu
    )
    assert result.pages[0].needs_review()
    assert len(Document(result.output).inline_shapes) == 1
