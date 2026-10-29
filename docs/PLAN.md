# Implementation and verification plan

## Product requirements

Import scanned PDFs. Preserve page order. Recognize printed text and attempt difficult
handwriting with a local vision model. Rebuild real Word tables, including explicit
rectangular merges. Preserve diagrams as crops and explain visible relationships,
labels and purpose using document context. Separate observation from interpretation.
Flag missing or uncertain content instead of inventing it. Export editable DOCX and
an auditable JSON project. No uploads, telemetry, automatic model downloads or cloud fallback.

## Iteration gates

| Iteration | Deliverable | Required evidence before next iteration |
|---|---|---|
| 1 | Data contracts, PDF rasterizer, image preparation, Tesseract, ruled tables, DOCX | Real image-only PDF OCR; table cells and merges in DOCX XML; resource bounds; invalid inputs |
| 2 | Local vision client and orchestration | Schema validation; loopback-only transport; redirects blocked; model errors, cancellation, recovery and resume tests |
| 3 | Qt desktop and review | Offscreen UI tests, source preview, corrected export, queue, pause/cancel, live logs |
| 4 | Delivery and acceptance | Whole test suite and lint; Windows CI and portable build; published test report with limitations |

No feature is described as verified on Windows or CUDA without an actual run there.
Mocks verify transport and orchestration; they do not measure model recognition quality.

Software gates 1–4 completed on 30 October 2026: 50 tests passed on Windows and Linux,
and the portable Windows app passed startup and actual DOCX export smoke checks.
Real-model CUDA quality and target-machine performance acceptance remain open; the
opt-in GPU test was explicitly skipped. See TESTING.md for the exact build and evidence.

## Architecture

`PySide6 desktop / CLI → sequential job runner → PDFium → OpenCV → Tesseract → local
llama.cpp vision (when selected) → validated page blocks → checkpoints → python-docx`.

Only one PDFium worker and one AI request run at a time. Each page is rendered and
released in turn. Disk-backed page images avoid keeping a complete document in RAM.
The UI reads image checkpoints, never calls PDFium from the GUI thread. Output is
written to a temporary sibling file and atomically moved into place. Existing files
are never overwritten implicitly. Checkpoints include input hash, settings and model
identity; interrupted jobs can resume completed pages. Review edits persist in the project.

## Quality and speed

- Fast: CPU OCR and ruled-table recovery. No semantic diagram claims.
- Balanced: CPU OCR plus vision for tables, visual candidates or low OCR confidence.
- Thorough: vision on every page, with previous-page context and OCR evidence.
- Handwriting setting forces vision in balanced/thorough mode.
- Low scores, blur, empty output, model disagreement, fallback and figures enter review.
- Printed OCR confidence and model self-ratings are distinct, uncalibrated signals.
- Model output is untrusted data. Text in scans is never executable instruction.

## Hardware budget

Initial reference: Qwen2.5-VL-7B-Instruct Q4_K_M and its matching projector via CUDA
llama.cpp. The model weights are about 4.68 GB; that is not total VRAM. Limit context,
image tokens, micro-batch and parallel slots. Reserve room for Windows and other apps.
Target conversion working set below 28 GiB total system use and 14 GiB VRAM, with
live telemetry and resource-pressure stops. Actual speed/VRAM require target hardware
benchmarks. An RTX 5070 Ti needs a recent CUDA build with Blackwell support.

## Acceptance corpus and remaining release gates

Run on Windows 11, 32 GB RAM, 16 GB RTX: clean print, English cursive, difficult
handwriting, Sinhala/Tamil mixed print, blur, rotation, two columns, borderless tables,
merged tables, flowcharts, accounting tables, low-contrast photos and 100+ page PDFs.
Compare against human ground truth with CER/WER and table cell accuracy; manually
score diagram grounding and fabricated claims. Record median/p95 time, peak RAM/VRAM,
model/build hashes, driver, settings and reviewer. User documents must never enter git.
Severely illegible scans, cross-page table continuation and exact facsimile layout are
not guaranteed. Dense page tiling and a specialized handwriting model remain future
quality upgrades until evaluated against that corpus.
