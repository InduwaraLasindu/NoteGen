"""Small offline ground-truth metrics. These measure transcription, not semantic truth."""


def distance(a, b) -> int:
    if len(a) < len(b):
        a, b = b, a
    previous = list(range(len(b) + 1))
    for i, item in enumerate(a, 1):
        current = [i]
        for j, other in enumerate(b, 1):
            current.append(min(current[-1] + 1, previous[j] + 1, previous[j - 1] + (item != other)))
        previous = current
    return previous[-1]


def transcription_metrics(expected: str, actual: str) -> dict:
    expected, actual = " ".join(expected.split()), " ".join(actual.split())
    # Bound the quadratic work. Benchmark per-page excerpts for large documents.
    if max(len(expected), len(actual)) > 5000:
        raise ValueError("Compare excerpts of at most 5000 characters for exact CER/WER")
    return {
        "cer": distance(expected, actual) / max(1, len(expected)),
        "wer": distance(expected.split(), actual.split()) / max(1, len(expected.split())),
        "normalization": "collapse whitespace; case and punctuation retained",
    }
