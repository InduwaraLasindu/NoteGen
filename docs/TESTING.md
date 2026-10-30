# Rust verification record

Implemented behavior, mocked/protocol tests and physical quality evidence are separate.
Use [Actions](https://github.com/InduwaraLasindu/NoteGen/actions/workflows/verify-and-build.yml)
for exact Windows/Linux results, native visual evidence and the executable download.

## Iteration gates

| Iteration | Evidence |
|---|---|
| 1 | 13 core tests + format/clippy: simulated 4/8/16/32 GiB profiles, busy VRAM, parameter validation, independent consensus and numeric disagreement |
| 2 | 31 tests: real PDFium/Tesseract raster-only PDF conversion, editable table, resume, DOCX merges/media, local SSE, faults/OOM/redirects/cancellation |
| 3 | 39 tests with all features/targets + format/clippy: Advanced tabs, text/table/figure review, light/dark/reduced motion, queue/worker lifecycle, bounded logs, human corrections |
| 4 | Additional real-model mixed-format regression, CPU-only isolation, manual selection, Balanced scheduling and draft-save protection; Windows/Linux/package gates in CI |

Before the execution environment disconnected, 43 ordinary tests passed locally with
the newer changes, and the actual native desktop started/rendered under a virtual X
display. Earlier iteration gates passed format/clippy. The final recovered source,
additional draft-save test, GPU lockfile and packaging are rechecked in Actions.

The native fixture contains **only raster image data**, known printed text and a
two-column table with cell 1250. No selectable PDF text shortcuts OCR. Rust draws the
fixture. NOTEGEN_FIXTURE_DIR exports its PNG/PDF for independent package tests.

## Real inference performed locally

- Official GLM-OCR Q8 + matching Q8 projector, llama.cpp b11998, CPU: the known printed
  sentence was recognized. Table Recognition on the source crop produced editable cells
  including 1250. Text-only whole-page mode omitted the table; NoteGen preserves table
  regions from other readers and uses the specialist table protocol.
- LightOnOCR-3 1B Q8 + matching F16 projector from richardyoung's pinned conversion,
  CPU: the full page returned the known text and HTML table. A crop returned Markdown
  inside HTML tags; the real test caught this and the parser now reconstructs its cells.
  The repeated live test passed. Grounding markers were absent in this sample, so the
  adapter emits an explicit source-grounding review warning.

These demonstrate native inference/adapters on synthetic print and tables. They do not
measure handwriting accuracy or diagram understanding. Model revisions/SHA256 are in
rust/downloads.json. Large model tests are opt-in and were invoked explicitly.

## CI/package verification

Both operating systems install PDFium/Tesseract, run Rust format, clippy with all
features/targets, and ordinary tests. Linux also converts the scan through the compiled
CLI, starts the real desktop and captures workspace/Advanced/review screenshots.
Windows builds release executables, verifies native runtime installation, packages all
required printed-OCR dependencies/licenses, and checks a raster PDF through the packaged
CLI for 1250 plus a real Word table. It starts the desktop extracted from the portable ZIP.

Screenshots show the actual Rust app in the test machine's CPU mode. They do not depict
an RTX machine or fabricated performance telemetry.

## Reproduce

Install Tesseract with eng/osd and PDFium chromium/8125; set NOTEGEN_PDFIUM to its library.

~~~shell
cargo fmt --all --check
cargo clippy --locked --all-features --all-targets -j 2 -- -D warnings
cargo test --locked --all-features --all-targets -j 2
cargo run --features desktop --example ui_capture -- build/workspace.png workspace
~~~

Use xvfb-run -a for native GUI capture on headless Linux. For actual AI testing, set
NOTEGEN_LIVE_ROOT, NOTEGEN_LIVE_ID (glm-ocr or lighton3-1b), NOTEGEN_LIVE_WEIGHTS,
NOTEGEN_LIVE_PROJECTOR and NOTEGEN_LIVE_SERVER, then:
cargo test --test engine live_model -- --ignored --nocapture.

## Remaining target-PC acceptance

No physical RTX GPU or access to your Windows PC was available. GPU-profile tests are
simulations. Actual layer-offload verification is implemented, but peak RAM/VRAM,
latency, driver compatibility and memory-pressure recovery need target-device runs.
The 4B/9B/27B catalog entries have not all been physically inferred in this session.

Evaluate a labeled private corpus of clear print, cursive, terrible handwriting, blur,
mixed scripts, merged/borderless tables, diagrams and cross-page context. Record CER/WER,
exact cell/topology accuracy, grounded diagram claims, false agreement, peak memory,
seconds/page and all model/runtime/driver hashes. Agreement is not calibrated accuracy;
correlated readers can share mistakes. Unreadable source information remains uncertain.

The Windows gate also invokes the real model installer and native CPU inference for
GLM-OCR, LightOnOCR-3 1B and Qwen3.5 2B, then a Maximum Accuracy conversion. Its generated
fixture now includes a bar chart; the vision adapter must preserve a valid source box,
visible description and contextual interpretation. See the run for whether these gates
have passed; inclusion in a workflow alone is not evidence of a successful test.
