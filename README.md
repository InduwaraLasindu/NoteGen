# NoteGen

**Scanned PDF → editable Word. Rust desktop. Local AI. Adaptive NVIDIA acceleration.**

NoteGen transcribes print and handwriting with local readers, rebuilds editable Word
tables, and preserves diagrams with observations and contextual interpretations.
The desktop and pipeline are Rust. PDFium, Tesseract and llama.cpp are native engines.
Python is not needed to run the current app.

![Native Rust desktop](docs/images/rust-workspace.png)

[Windows installer](https://github.com/InduwaraLasindu/NoteGen/releases/latest)
· [Portable builds](https://github.com/InduwaraLasindu/NoteGen/actions/workflows/verify-and-build.yml)
· [Setup](docs/WINDOWS.md) · [Verification](docs/TESTING.md) · [Iteration plan](docs/RUST_PLAN.md)

[Download NoteGen-Setup-0.2.0.exe](https://github.com/InduwaraLasindu/NoteGen/releases/download/v0.2.0/NoteGen-Setup-0.2.0.exe)
(about 61 MB), run it, and follow the wizard. It installs for your Windows account,
adds Start menu shortcuts and an optional desktop shortcut, and includes an uninstaller.
[Release page](https://github.com/InduwaraLasindu/NoteGen/releases/tag/v0.2.0) · [Installer guide](docs/INSTALLER.md)

English printed OCR and Word export work immediately. For local AI, choose
**NoteGen → Set up local AI** in the Start menu. The setup downloads the appropriate
runtime and model weights for this PC. Restart NoteGen afterward. No Python or Rust
installation is needed. Models and document data survive upgrades/uninstall; a selected
CUDA/Vulkan runtime survives upgrades.

The [installer acceptance run](https://github.com/InduwaraLasindu/NoteGen/actions/runs/41185176403) passed actual installation,
GUI launch, PDF rasterization/OCR, editable Word-table export, same-version upgrade,
shortcut/registry registration, uninstall and data preservation. The original
[application verification](docs/TESTING.md) passed 65 tests per OS and real CPU AI conversion.
The release includes SHA256 and provenance files. The installer is unsigned.

Portable ZIPs remain available under the latest successful **Rust verification** workflow's
**NoteGen-Rust-Windows-x64** artifact. Extract both ZIP layers and keep the folder together.

Setup selects models for actual free memory, downloads pinned revisions, verifies SHA256
and registers them. Models are separate, potentially large downloads. Once installed,
conversion uses only local files and loopback HTTP. There is no cloud fallback, API key
or document upload.

## Features

| Capability | Behavior |
|---|---|
| Adaptive hardware | CPU/SIMD, RAM and CUDA/Vulkan discovery; strongest fitting installed readers; memory re-admission before model load |
| Four presets | Fast printed OCR without loading/checksumming AI assets; Balanced with selective vision; Accuracy on every page; Maximum Accuracy with independent families and difficult-region crop retries |
| Print and handwriting | Tesseract baseline, LightOnOCR-3 / GLM-OCR specialists and Qwen vision; original source and all alternative readings retained |
| Ensemble decisions | Weighted medoid selects an observed transcript; duplicate models/crops do not count as independent agreement; numeric conflicts require review |
| Tables | Native ruled-grid recovery and HTML/Markdown/JSON tables; real Word cells, merges and repeated headers |
| Diagrams | Source crop, visible evidence and qualified contextual interpretation kept separately; interpretations require review |
| Scan preparation | Orientation detection, small-angle deskew, contrast normalization, blur warnings and image memory limits |
| Live activity | Stage/page progress, elapsed/estimated time, filterable activity, NG error codes and remedies; full JSONL/native runtime logs |
| Durable jobs | Queue, page ranges, pause, active-request cancellation, input/model/settings-bound checkpoints, restart recovery and protected output names |
| Review | Side-by-side source boxes, editable text/cells/diagram explanations, reader evidence, validated page JSON and persistent human corrections |
| Minimal native UI | Scanning-page motion graphic, orbiting accents, eased hover/preset transitions, progress sheen, completion check and animated drawers/windows; configurable in Advanced |
| Appearance controls | Light/dark, accent/background, scale, corners, speed, transition timing, orbit density, active/idle FPS, glow and reduced motion; live appearance preview while converting |
| Offline CLI | Diagnostics, hardware plan, registration, conversion, project inspection, native export and re-export |

Windows UI rendering uses wgpu/Direct3D or Vulkan; AI uses CUDA/Vulkan through llama.cpp.
Actual offloaded layers are recorded in the native startup log. PDF rendering, Tesseract,
export and some preprocessing remain CPU operations. Not every operation can occupy
100% of the GPU. The UI and AI select graphics devices independently.

[Actual native motion preview](docs/images/notegen-motion.mp4). Open **Advanced → Appearance**
to customize it. Reduced motion freezes decorative graphics and transitions. Ambient
animation sleeps when unfocused; a running job still updates its status. The live log
drawer animates and virtualizes rows so long jobs do not build thousands of labels.

## Hardware and models

| VRAM | Illustrative automatic choice, when installed and enough memory is free |
|---|---|
| 4 GB | LightOnOCR-3 1B Q8 + Qwen3.5 2B Q4 |
| 8 GB | Small OCR specialist + Qwen3.5 4B Q5 |
| 16 GB | LightOnOCR-3 4B Q8 + Qwen3.5 9B Q5 |
| 32 GB | LightOnOCR-3 4B Q8 + Qwen3.8 27B Q4 |

Maximum adds a different OCR family such as GLM-OCR when it fits. One model is resident
at a time. Pages are processed per model before switching, avoiding repeated weight
loads for every page. Windows/UI memory headroom is reserved. Threads, resolution,
image tokens, context, batch and micro-batch adapt; Advanced supports manual model
selection and validated overrides. Estimates are planning inputs, not measured peaks.

Catalog checked **30 October 2026**: LightOnOCR-3, GLM-OCR, Qwen3.5 and Qwen3.8.
[Primary sources](docs/SOURCES.md) distinguish vendor models from community GGUF
conversions. Setup pins runtime b11998 and immutable model revisions/checksums.

## Quality and verification

Real PDFium/Tesseract conversion and actual GLM-OCR/LightOnOCR-3/Qwen3.5 2B CPU inference have
been exercised on a raster-only printed scan and accounting table. Tests cover model
protocols, faults/cancellation, memory profiles, consensus, persistence, Word structure
and native desktop rendering. The current source passes 65 ordinary Rust tests,
format and clippy locally. Windows/Linux CI compiles the executables and smoke-tests
the portable package. See [exact evidence and limits](docs/TESTING.md).

Terrible handwriting and severe blur can destroy information. Agreement is not a
calibrated accuracy score; AI may misread text or a diagram. Flagged content and diagram
interpretations need source review. Broad handwriting accuracy and actual peak RAM/VRAM,
speed and offload on 4/8/16/32 GB RTX PCs still require target-hardware/corpus measurement.
Word export prioritizes editable structure and reading order over a pixel-identical copy.

## Build

~~~shell
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy
cargo build --locked --release --features desktop --bins
cargo run --locked --bin notegen-cli -- doctor
cargo run --locked --features desktop --bin NoteGen
~~~

Tests require Tesseract and PDFium; set NOTEGEN_PDFIUM to the native library.
[Development](CONTRIBUTING.md). The [Python predecessor](legacy/python) is archived and
is not part of the Rust release. NoteGen is MIT-licensed; dependency/model terms apply.
