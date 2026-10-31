# Windows setup and usage

Use Windows 10/11 x64. Download the latest successful **Rust verification and Windows
executable** run from [Actions](https://github.com/InduwaraLasindu/NoteGen/actions/workflows/verify-and-build.yml),
then choose **NoteGen-Rust-Windows-x64**. Extract the Actions artifact ZIP and portable
ZIP inside it. Keep the NoteGen folder together. English print works immediately;
the native runtime, PDFium and Tesseract accompany the unsigned executables.

## One-time AI setup

Open PowerShell inside the extracted NoteGen folder:

~~~powershell
.\scripts\Setup-Runtime.ps1 -Backend Auto
.\scripts\Install-Models.ps1 -MaximumAccuracy
.\NoteGen.exe
~~~

Auto selects CUDA13 on NVIDIA, Vulkan otherwise. Update the NVIDIA driver first.
CUDA13 uses CUDA 13.4 and requires a compatible GPU/driver; older cards may need
-Backend CUDA12 (CUDA 12.4) or Vulkan. RTX 50-series needs a Blackwell-compatible
runtime such as CUDA13. CPU mode is also available with -Backend CPU.
CUDA runtime/server downloads total roughly 590 MB; Vulkan roughly 38 MB.

Scripts pin official llama.cpp b11998 asset SHA256. Model setup pins immutable model
revisions and original LFS SHA256, probes free RAM/VRAM, chooses fitting models and
registers verified files. Conversion never downloads. Downloads resume via .part files.
If a range is rejected, remove only that invalid .part and retry. Model terms remain
applicable; some quantizations are community conversions. Publishers are recorded in
assets/model-downloads.json (source: rust/downloads.json).

If Windows blocks a downloaded script, inspect it and Unblock-File that script.
A process-only policy for an inspected script is available:

~~~powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\Setup-Runtime.ps1
~~~

No account or permanent execution-policy change is needed. Specific model selection:

~~~powershell
.\scripts\Install-Models.ps1 -ModelIds glm-ocr,lighton3-1b,qwen35-2b
~~~

## Convert, inspect and correct

1. Choose/drop PDFs, an output folder and optional page range (1-4,8).
2. Choose Fast, Balanced, Accuracy or Maximum accuracy. Enable handwriting for
   handwritten pages. Maximum trades time for additional independent readings/crops.
   Enabling handwriting from Fast selects Accuracy; Fast is printed OCR only.
3. Convert and open Live activity. Pause acts at safe checkpoints; cancellation
   also aborts an active inference request. Completed evidence remains on disk.
4. Open Review results. Compare source boxes with transcripts; edit text, table cells,
   visible diagram evidence and interpretation. Page JSON allows merges, headers,
   source boxes and structural edits with validation.
5. Save drafts or mark pages reviewed, then export a new corrected Word document.

Add factual course/document context in Advanced → Recognition for diagram interpretation.
Every diagram interpretation requires review. Severe blur, illegibility and numeric
conflicts remain explicit. Models cannot reliably restore information absent in a scan.

Advanced exposes backend/device, threads, memory limits/reserves, GPU layers, Flash
Attention, projector placement, KV precision, image/context/output tokens, batch sizes,
timeout, rendering/crops, agreement/review thresholds, sampling/seed, languages,
preprocessing and Word typography. Zero means adaptive where labeled. Models supports
automatic or manual selection, GGUF registration and runtime paths. All parameters
exposes validated JSON; settings import/export is supported. Appearance controls theme,
colors, scale, rounding and compact spacing. Motion controls include scan/orbit graphics,
speed, transition timing, density, glow, active/idle FPS and reduced motion. Preview
appearance while a conversion runs; changing recognition parameters waits for the job.

Require verified acceleration to fail explicitly if model layers did not offload.
The UI renderer and AI backend are independent. CPU stages still use CPU resources.

## CLI and manual installation

Run the CLI from the NoteGen folder. Select a compatible GGUF and matching projector:

~~~powershell
.\notegen-cli.exe register --id glm-ocr --weights C:\Models\GLM-OCR-Q8_0.gguf --projector C:\Models\mmproj-GLM-OCR-Q8_0.gguf
.\notegen-cli.exe doctor
.\notegen-cli.exe plan --catalog
.\notegen-cli.exe convert C:\Scans\notes.pdf --preset maximum --handwriting -o C:\Scans\notes.docx
~~~

Export Advanced settings and pass --settings settings.json before a subcommand to
override parameters. Registry descriptors end in .notegen.json; unrelated Hugging Face
metadata is ignored. Changed model bytes require re-registration. An air-gapped PC can
use copied runtimes/models; re-register paths after moving them.

External-server mode accepts only literal loopback HTTP. Its /v1/models alias must
match the registered model. Managed mode is required to verify GPU allocation and
normally handles independent model switching automatically.

## Logs, recovery and errors

Projects live under the output folder's .notegen: source PNGs, page JSON, candidates,
settings/model identity, activity JSONL, hardware/plan and native runtime logs. Back up
the entire project to preserve corrections. Matching work resumes from checkpoints.
Recognition changes create distinct projects; UI appearance and Word typography do not
invalidate recognition. Existing output files are protected.

| Code | Action |
|---|---|
| NG100 | Choose a readable, unlocked PDF and valid page range |
| NG102 | Correct Advanced values or restore defaults |
| NG110 | Check Tesseract and requested language data |
| NG200 | Check runtime/backend, model/projector paths and native startup log |
| NG201 | Review invalid/truncated responses and retained alternatives |
| NG202 / NG300 | Free RAM/VRAM, reduce buffers/model and resume |
| NG203 | Use a literal loopback endpoint; remote hosts/redirects are blocked |
| NG400 / NG401 | Check disk access, close Word or choose a new output name |
| NG500 | Inspect project/checkpoint identity or worker failure evidence |
| NG900 | Cancelled; completed work remains available |

## Source build

Install Rust 1.99.1 MSVC and Microsoft C++ Build Tools. Build the GUI/CLI with
cargo build --locked --release --features desktop --bins.
Run scripts/Build-Windows.ps1 with Tesseract installed to package the app.
CI tests native scans on Linux/Windows, verifies the CPU runtime installer, checks
packaged Word/table output and starts the executable extracted from its ZIP.

Human edits use separate page-*.edits.json overlays. Later AI checkpoints cannot overwrite
those corrections; back up both the original page evidence and edit overlays together.
