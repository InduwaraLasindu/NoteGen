# Verification record

30 October 2026. Tests are gates, not evidence of perfect OCR or diagram truth.

| Gate | Result |
|---|---|
| Iteration 1: core conversion | 18 passed; real Tesseract on a generated raster-only PDF |
| Iteration 2: AI transport and orchestration | 36 passed; includes deterministic model responses and error paths |
| Iteration 3: desktop and review | 44 passed; includes a real OCR conversion through the Qt worker |
| Iteration 4: local regression suite | 48 passed, 1 GPU test skipped |
| Windows source tests | 48 passed, 1 GPU test skipped in GitHub Actions; Python 3.12.15 |
| Packaged Windows app / clean Linux CI | In progress; see Actions for build results |
| NVIDIA CUDA + real model quality | Not run in this CPU-only development environment |

The current suite checks PDF bounds and invalid inputs, word boxes, table cells,
horizontal DOCX merges, repeated headers, figure image/alt text, literal text safety,
output collision protection, page selection, fingerprints, checkpoint corruption,
cancel/resume, pause release, RAM/VRAM pressure, explicit missing-engine handling,
vision-only conversion, loopback-only networking, proxy bypass prevention, blocked
redirects, SSE validation, truncation, invalid model data, one-retry limits, streamed
server errors, numeric disagreement, UI queue deduplication, editable text/tables/
figures, automatic review invalidation, saved-project reopening and smaller desktop layout.

## Run locally

```powershell
$env:QT_QPA_PLATFORM='offscreen'
python -m pytest -q -rA
python -m ruff check src tests scripts
python -m ruff format --check src tests scripts
```

Real OCR tests require Tesseract and `eng` data. A skipped test is not a pass.
The opt-in GPU test uses an already running local model; it downloads nothing:

```powershell
$env:NOTEGEN_RUN_GPU='1'
python -m pytest -m gpu -v
```

That test checks a synthetic table's number and a labeled water-cycle arrow diagram.
It is a smoke test, not a handwriting benchmark. Keep a human-reviewed private corpus
for difficult handwriting, blur, Sinhala/Tamil and complex tables. The procedure and
required measurements are in [PLAN.md](PLAN.md).

## Measure on the target RTX PC

```powershell
python scripts/benchmark.py sample.pdf --output sample-notes.docx --report benchmark.json --mode thorough
```

Optional `--expected ground-truth.txt` computes exact CER/WER for excerpts up to 5000
characters. Whitespace is normalized, while case/punctuation remain significant. Table
accuracy and diagram grounding require separate human scoring. Reports record page
times, sampled app RSS, free system RAM and GPU memory (including other GPU apps).
The external model server's CPU working set is not included in app RSS. Record model
SHA256, runtime release/build, driver, peak server memory and observer judgments separately.

No universal speed, accuracy or peak-memory number is claimed until that validation runs.

The first clean Linux CI run found a missing system `libEGL` dependency, and local
packaged-export validation found a python-docx template lookup issue. CI installs
the native Qt libraries explicitly, and both portable executables collect the full
docx package. The package smoke test exports actual text, table cells and a source
figure; starting the EXE alone would not catch that failure.
