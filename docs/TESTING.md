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
| 5 | 61 ordinary tests + format/clippy locally: motion/reduced-motion render comparisons, animation timing and frame budgets, backward-compatible appearance settings, invalid AI grounding recovery and raw response evidence |
| 6 | 65 ordinary tests + format/clippy locally: Fast conversion with damaged/invalid unused AI registrations, selected-reader checksum rejection, handwriting/preset validation, CLI settings preservation and honest single-family agreement labels |

The current source passes all 65 ordinary tests with all features/targets on Linux,
plus strict format and clippy checks. The native desktop was started under an authenticated
virtual X display; workspace, Advanced and review were captured, and the actual scanning
graphic was recorded for four seconds. The screenshot harness decodes its output before
reporting success. A compiled-CLI raster-PDF conversion produced an editable Word file.
Cross-platform and packaged-executable status is recorded by the Actions run, independently
of these local checks.

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
- Qwen3.5 2B Q4_K_M + matching F16 projector, b11998 CPU: full-page and crop inference
  passed in the explicit live test. It read 1250, rebuilt the table and described the
  bar chart as a visual representation of sales. Its JSON supplied zero-sized boxes.
  The regression now retains text/table data without invented coordinates and attaches
  an ungrounded figure to the full source page with an uncertainty/review warning.
  Valid coordinates remain unchanged; malformed table structure is still rejected.
  Raw reader responses are retained separately under the local project/runtime
  `responses` directory for diagnosis, without putting document text in live log messages.
- A release-CLI Maximum Accuracy conversion completed with Tesseract, LightOnOCR-3,
  Qwen3.5 and GLM: four full-page families plus three crop readings. Native Word XML
  contains the reconstructed table (1250) and two retained source images. This run
  took 197.69 seconds on CPU with eight inference threads and a resumed printed-OCR
  checkpoint; it is not a cold-start benchmark. Agreement was 84.0%, and source/grounding
  warnings correctly required review. The native review capture shows this real result.

These demonstrate native inference/adapters on synthetic print, tables and one simple
chart. They do not measure broad handwriting accuracy or diagram understanding. Model revisions/SHA256 are in
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
NOTEGEN_LIVE_ROOT, NOTEGEN_LIVE_ID (glm-ocr, lighton3-1b or qwen35-2b), NOTEGEN_LIVE_WEIGHTS,
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
