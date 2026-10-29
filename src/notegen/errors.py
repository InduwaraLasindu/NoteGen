"""Stable error codes and bounded, actionable diagnosis; no generated shell fixes."""

from dataclasses import dataclass


@dataclass(frozen=True)
class Diagnosis:
    title: str
    cause: str
    action: str
    retryable: bool = False


CATALOG = {
    "NG100": Diagnosis(
        "Invalid input",
        "The PDF cannot be read or opened.",
        "Choose a valid PDF. Unlock encrypted files in your PDF editor first.",
    ),
    "NG101": Diagnosis(
        "Page too large",
        "The PDF exceeds a configured size or page limit.",
        "Split the PDF, or use a lower rendering DPI.",
    ),
    "NG110": Diagnosis(
        "OCR unavailable",
        "Tesseract or the selected language data is missing.",
        "Install Tesseract and the language data, then set its executable path.",
    ),
    "NG111": Diagnosis(
        "OCR failed",
        "The OCR process failed or exceeded its time limit.",
        "Check the selected languages and try a cleaner scan or lower DPI.",
        True,
    ),
    "NG200": Diagnosis(
        "Local AI unavailable",
        "The loopback vision server is not responding.",
        "Start your local llama.cpp vision server, then retry.",
        True,
    ),
    "NG201": Diagnosis(
        "Invalid AI output",
        "The model returned incomplete or invalid page data.",
        "Review the preserved OCR result. Retry with a clearer scan or another model.",
        True,
    ),
    "NG202": Diagnosis(
        "GPU memory pressure",
        "The server reported memory or context exhaustion.",
        "Close GPU-heavy apps. Use Q4 weights, fewer image tokens or a smaller context.",
        True,
    ),
    "NG203": Diagnosis(
        "Remote endpoint blocked",
        "The endpoint is not a literal loopback address.",
        "Use http://127.0.0.1:8080 or http://[::1]:8080. Remote hosts are disabled.",
    ),
    "NG204": Diagnosis(
        "Vision model required",
        "The running model does not advertise image input.",
        "Load a supported vision GGUF and its matching multimodal projector.",
    ),
    "NG300": Diagnosis(
        "Memory pressure",
        "Too little free RAM or VRAM remains.",
        "Close other apps and resume. Reduce image size or model memory use.",
        True,
    ),
    "NG400": Diagnosis(
        "Cannot save",
        "The output is locked, unwritable, or the disk is full.",
        "Close the Word file and choose a writable folder with free space.",
        True,
    ),
    "NG401": Diagnosis(
        "Output exists",
        "An existing file would be replaced.",
        "Choose a new filename. Existing files are protected.",
    ),
    "NG500": Diagnosis(
        "Checkpoint mismatch",
        "The input, model, or settings changed.",
        "Start a new conversion; use the original settings to resume this project.",
    ),
    "NG900": Diagnosis(
        "Cancelled",
        "The conversion was stopped by you.",
        "Completed pages are saved. Start the same job to resume.",
    ),
    "NG999": Diagnosis(
        "Unexpected error",
        "An unexpected internal error interrupted the operation.",
        "Open the local activity log and report the error code and app version.",
    ),
}


class NoteGenError(Exception):
    def __init__(self, code: str, detail: str = ""):
        self.code = code if code in CATALOG else "NG999"
        self.detail = detail
        self.diagnosis = CATALOG[self.code]
        super().__init__(
            f"{self.code}: {self.diagnosis.title}" + (f" — {detail}" if detail else "")
        )


def classify(exc: Exception) -> NoteGenError:
    if isinstance(exc, NoteGenError):
        return exc
    if isinstance(exc, MemoryError):
        return NoteGenError("NG300")
    if isinstance(exc, OSError):
        return NoteGenError("NG400", type(exc).__name__)
    return NoteGenError("NG999", type(exc).__name__)
