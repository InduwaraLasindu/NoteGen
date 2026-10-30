# Rust rewrite: implementation and acceptance

The Rust app replaces Python at runtime. PDFium, Tesseract and llama.cpp remain native
engines. No documents are uploaded. Hardware acceleration means actual CUDA/Vulkan
offload in a compatible llama.cpp build and hardware-rendered UI; CPU-only stages
remain CPU stages. GPU discovery and successful executable startup alone do not prove
that model layers ran on a GPU. The server capability/startup log and a target-PC run
are required evidence. No universal accuracy or zero-error claim is made.

## Iterations

1. Typed contracts, hardware discovery, resource planning, independent consensus.
   Gate: 13 Rust tests passed locally. Simulated 4/8/16/32 GiB profiles, busy GPU,
   invalid parameters, merge invariants, remote endpoint rejection, duplicate-reader
   protection and numeric disagreement. Planning is deterministic and does not need a GPU.
2. Native PDF/image/OCR adapters, local model lifecycle, model-specific protocols,
   high-resolution crop retries, persistent evidence, resumable pipeline and DOCX.
   Gate passed: 31 automated tests (13 core + 18 engine), format and clippy. Native
   raster-only PDF + real Tesseract conversion/resume; local HTTP streaming, faults,
   cancellation, source graphics, merges and Word XML. Separately ran GLM-OCR Q8 on
   CPU against a generated scan and a table crop: text recognized and cell 1250
   recovered with the model-specific table protocol. No GPU/handwriting claim.
3. Minimal Rust desktop, Advanced settings, animations/customization, review/editing.
   Gate passed: 39 tests across core, native engine and desktop; format and clippy
   with all features/targets. Every Advanced tab, text/table/figure review, light/dark
   and reduced motion, queue recovery, worker failure, bounded live logs, safe output
   names and saved human corrections. Fixed texture-size handling for small adapters.
4. Windows release, clean Linux/Windows CI, package smoke tests and hardware acceptance.
   Gate: tests + format + clippy + actual compiled CLI conversion/export and desktop startup.

Every gate must pass before the following stage is declared complete. GPU quality tests
and private handwriting corpus acceptance are distinct from mocked/protocol tests.

## Accuracy policy

- Fast, Balanced, Accuracy and Maximum Accuracy presets. Handwriting requests force
  AI reading or an explicit setup error, never quiet success from printed OCR alone.
- Maximum uses a fitting OCR specialist, a different OCR family, and a general vision
  reader sequentially. One model stays resident at a time; process all target pages
  before switching weights to avoid reloading every model for every page.
- Select an observed transcript using weighted agreement. Never average numeric values
  or splice unobserved words together. Repeated seeds/crops of identical model families
  do not count as independent agreement. Preserve numeric conflicts and alternatives.
- Enlarge source crops for low agreement, handwriting and dense small text. Retain
  original scans, candidate readings, source boxes, warnings and reviewed states.
- Diagram observation and contextual interpretation remain separate and require review.

## Adaptive compute policy

Probe CPU/SIMD, RAM available, NVIDIA adapters and actual free VRAM. Select the strongest
installed task-appropriate models that fit estimated combined weights/projector/context/
workspace needs. Re-probe before model admission. Configure CPU threads, GPU index/layers,
Flash Attention, image tokens, context, batch/micro-batch and rendering/crop resolution.
Use available compute while reserving OS/UI memory headroom. Live OOM recovery reduces
working memory or chooses a fitting reader and records the change. Multi-model reads
are serial under a shared VRAM budget. More VRAM does not inherently guarantee better
transcription; ranks are curated choices, not measured accuracy percentages.

## Model catalog checked 30 October 2026

- LightOnOCR-3 1B/4B, released October 2026: https://huggingface.co/lightonai/LightOnOCR-3-4B
  and https://github.com/lightonai/LightOnOCR . Respect image-first `grounding` protocol;
  normalized boxes are 0–1000. Do not force a general chat schema onto this OCR specialist.
- GLM-OCR 0.9B: https://huggingface.co/zai-org/GLM-OCR and official runtime conversion
  https://huggingface.co/ggml-org/GLM-OCR-GGUF . Use `Text Recognition:` for text OCR.
- Qwen3.5 2B/4B/9B for small/medium GPUs; Qwen3.8-27B for larger GPUs:
  https://huggingface.co/Qwen/Qwen3.8-27B and https://github.com/QwenLM/Qwen3.8 .
  Use a current compatible GGUF runtime and matching projector. Published vendor scores
  are not NoteGen quality evidence. Quantizations and peak memory need local validation.

## Remaining physical acceptance

Run the built application on the target RTX PC with a labeled corpus of print, ordinary
cursive, difficult handwriting, Sinhala/Tamil mixed print, blur, tables and diagrams.
Record CER/WER, exact table cells and topology, grounded diagram claims, false agreement,
peak RAM/VRAM, per-page latency, driver/runtime and model SHA256. Test real 4/8/16/32 GiB
devices or equivalent controlled free-memory cases. Unit profiles cannot replace these runs.

## Release recovery and final gate

The development environment disconnected during final packaging. The tested source was
recovered into GitHub. CI regenerates the native GPU lockfile, formats source, runs all
ordinary tests/clippy, captures native UI evidence, and compiles/tests the Windows ZIP.
Final completion depends on successful Windows/Linux jobs, not an inferred build result.
