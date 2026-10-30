# Run NoteGen on Windows 11

## Option A: portable app

When the **Verify and build Windows** workflow passes, open the workflow run in the
repository's Actions tab and download the **NoteGen-Windows-x64** artifact. Extract
both ZIP layers. Keep the whole `NoteGen` folder together and run `NoteGen.exe`.
It is an unsigned early build. The portable app contains the Python/Qt runtime;
Tesseract, llama.cpp and model weights are installed separately. Do not move the EXE
out of its folder. No administrator permission is needed to run NoteGen itself.

## Option B: run from source

Install Python 3.12 x64 from [python.org](https://www.python.org/downloads/windows/).
In a PowerShell terminal opened at the repository root:

```powershell
.\scripts\Setup-Windows.ps1 -Launch
```

The setup script downloads Python packages once. Later launches work offline:

```powershell
.\scripts\Run-NoteGen.ps1
```

The scripts do not change system execution policy. If your machine blocks local
PowerShell scripts, use Python directly in a terminal:

```powershell
py -3.12 -m venv .venv
.\.venv\Scripts\python.exe -m pip install -r requirements-lock.txt
.\.venv\Scripts\python.exe -m pip install --no-deps --no-build-isolation -e .
.\.venv\Scripts\python.exe -m notegen.app
```

## Install printed OCR

Follow [Tesseract's Windows installation guidance](https://tesseract-ocr.github.io/tessdoc/Installation.html).
Its documentation links Windows builds maintained by UB Mannheim. Standard install
location: `C:\Program Files\Tesseract-OCR\tesseract.exe`.
Use the app's Setup tab to select another location if needed. Install `eng` and `osd`
language files. For Sinhala or Tamil print, also install `sin` / `tam`, and choose
`eng+sin` / `eng+tam`. Recognition quality varies; those languages need acceptance tests.

## Install local vision AI for RTX

1. Use a recent NVIDIA driver and a **CUDA-enabled Windows x64** build from the
   [official llama.cpp releases](https://github.com/ggml-org/llama.cpp/releases).
   Include its matching CUDA runtime libraries. RTX 50-series cards need a build
   supporting Blackwell; confirm the CUDA device is listed in the server console.
2. Download [Qwen2.5-VL-7B-Instruct Q4_K_M](https://huggingface.co/ggml-org/Qwen2.5-VL-7B-Instruct-GGUF)
   and the **matching projector** from the same model repository. The main weights
   are about 4.68 GB; the projector, KV cache and image encoder use additional memory.
3. Start the server from a terminal, using your actual file paths:

```powershell
.\scripts\Start-LocalAI.ps1 `
  -Server 'D:\NoteGen\llama\llama-server.exe' `
  -Model 'D:\NoteGen\models\Qwen2.5-VL-7B-Instruct-Q4_K_M.gguf' `
  -Projector 'D:\NoteGen\models\mmproj-Qwen2.5-VL-7B-Instruct-Q8_0.gguf'
```

The default endpoint is `http://127.0.0.1:8080` and the alias is `notegen-vision`.
Keep the server terminal open. Press **Check local engines & hardware** in NoteGen.
The model must advertise `image` input through `/v1/models`. A text-only server cannot
describe diagrams. Use a recent compatible llama.cpp build if diagnostics report NG204.

Start with 16,384 context tokens, 3,072 image tokens, 128 micro-batch tokens and one
parallel request. These are budget-conscious starting settings, not a measured memory
guarantee. For memory errors try `-ImageTokens 2048` or `-ProjectorOnCPU`; the latter
reduces GPU memory at a speed cost. If the context becomes too small, do not force
truncation: retain OCR and review instead. Model weights are never auto-downloaded by
the conversion app, and the server launcher sets offline mode.

## Convert and review

Add or drop PDFs → choose profile/languages → optionally enter subject context →
Convert queue → Review and correct → Export corrected Word.

- **Fast**: CPU printed OCR and ruled tables. Semantic diagram descriptions need AI.
- **Balanced**: AI for low-confidence pages, tables, detected visual regions or blur.
- **Thorough**: AI on every page. Use this for complex layouts or difficult writing.
- **Expect handwriting**: requires a local vision model and prevents silent CPU-only use.
- Page ranges accept `1-4,7,9`. Each queued document uses that range independently.
- A diagram's **Observed** text describes visible evidence; **Interpretation** gives
  a qualified explanation. Verify it against the scan. It is not guaranteed truth.
- Review figures and uncalibrated low confidence scores. `[illegible]` means information
  cannot be recovered reliably. Severe blur cannot be reversed reliably by software.
- Text and table edits autosave when you change regions. Mark a page reviewed only
  after checking it. The original Word export is protected; corrected export uses a new name.
- Double-click a completed queue item to reopen that document's saved review.
- Pause takes effect at the next safe boundary. Cancel stops OCR promptly and AI at
  the next stream chunk or read timeout (default 45 s). Completed pages survive.
- Retry the same PDF/settings to reuse completed checkpoints. Reopen a project folder
  in Review after restarting the app. A changed input hash/settings/model metadata gets
  a different cache. If replacing weights under an identical alias/metadata, use a new
  alias or new project to avoid reusing old recognition.

Default output: Documents\NoteGen. Checkpoints and activity logs are under `.notegen`
inside the output folder. These contain document images and recognized text; keep
them private as you would the original scan. NoteGen does not upload or delete them.
The live log records processing stages, retries and error codes without OCR text.
Memory figures are best-effort telemetry. GPU data comes from `nvidia-smi`; inference
still works if telemetry is unavailable. The app keeps one page and one AI job active.

## Command line

```powershell
notegen doctor
notegen convert scan.pdf -o notes.docx --mode thorough --handwriting --context 'Biology notes'
notegen convert scan.pdf -o notes.docx --mode fast --language eng+sin --pages 1-5
notegen export 'D:\Notes\.notegen\my-project' -o corrected.docx
notegen --debug convert scan.pdf -o diagnostic-notes.docx --mode fast
```

The portable CLI is `CLI\NoteGenCLI.exe`. The source install uses `notegen` or
`python -m notegen`. Existing DOCX files are never overwritten automatically.
The optional `--debug` traceback stays local and may contain local file paths.

## Developer build

```powershell
python -m pip install -r requirements-dev-lock.txt
python -m pip install --no-deps --no-build-isolation -e .
.\scripts\Build-Windows.ps1
```

Read [the test report](TESTING.md) before treating this as a validated production OCR engine.
