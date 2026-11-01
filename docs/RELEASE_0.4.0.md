# NoteGen 0.4.0

This release continues the restored Rust application. It retains the 0.3 writer profiles, confirmed handwriting examples, clarification, editable Word tables, diagram crops, font packs, checkpoint recovery and independent reader comparisons.

## AI installation in the installer

The Windows wizard offers **Install local AI and all required models for this PC**. It is selected for a new interactive installation. The wizard installs the native dependencies, then detects available hardware and installs the fitting Maximum reader set, including independent OCR and vision families. It shows verified download and activation progress. Cancellation preserves resumable downloads and the working installation. Setup success is reported only after runtime probing and model checksum verification.

Models are downloaded from their publishers, rather than embedded as many gigabytes in the EXE. Internet and free disk space are needed for AI setup. Uncheck the AI option for an offline installation; printed OCR works immediately. The same complete setup is available in Settings. Incompatible larger catalog models are not required and are not downloaded automatically.

Silent deployment stays offline unless `/AI=1` is supplied:

```powershell
.\NoteGen-Setup-0.4.0.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /AI=1 /AIBACKEND=auto
```

Optional `/AIPRESET=maximum` (the default) and `/AIBACKEND=cpu|cuda|vulkan|auto` select deployment preferences. AI setup failure or cancellation returns installer exit code 10, with the base app installed and an error in the installer log. Retry in Settings to resume.

## Updates

Settings > Updates & installation checks all installed models, the native AI runtime and NoteGen releases. Checks use official publisher metadata; downloads are immutable-revision or release pinned and SHA-256 verified. Optional idle checks run at most daily. Installation is explicit. Active versions are retained until replacements pass verification, with model rollback in Studio and runtime rollback in Settings. Healthy newer versions are reused by setup.

PDFium, Tesseract, English language data and the Visual C++ runtime are serviced together through NoteGen installer updates to preserve binary compatibility. A verified app update can be downloaded and launched from Settings. For a private GitHub repository, set the `NOTEGEN_GITHUB_TOKEN` environment variable to a token with access to release metadata/assets. It is sent only to the NoteGen GitHub API, is not saved in configuration, and is not sent to model publishers. A failed check is shown as an error, never as “up to date.”

## Hardware test

Settings > Hardware tests a small set of CPU worker counts with repeatable synthetic processing, then runs up to two local AI recognition probes when a managed reader is installed. It checks the exact number 1250 and reports whether GPU offload was actually confirmed. Each AI probe is capped at 75 seconds, and the entire task is cancellable. Failed or incorrect trials cannot win. The fastest correct AI trial supplies the proposed worker count; otherwise the report clearly labels the CPU-only measurement. Adaptive memory/model selection continues to account for available RAM/VRAM. Quality and image-resolution preferences are preserved. Apply or undo the recommendation and save the report from the same screen.

The test uses a generated scan and a system font, never a private document. It estimates useful settings under the current system load; it does not prove optimal settings for every document.

## Live AI view and settings

The live panel displays the current source page, the crop being re-read, the active reader and streamed draft recognition. Drafts are provisional until parsing, independent comparison and human review. Reasoning channels are not read, and reasoning-tag sections are hidden. Preview drafts are ephemeral UI events, not activity-log transcripts. Disable or clear the panel in Settings.

Everyday settings have explanations, while Advanced retains all power-user controls and validated JSON import/export. A searchable reference describes every serialized setting, including appearance, local runtime, memory budgets, writer examples and context. UI/update preferences do not invalidate existing recognition checkpoints.

The master progress bar estimates total remaining work across the queue, model loads, readers, crop retries and export. Estimates adapt to observed stage timings, suspend during pause/clarification and reach 100% only on success. Downloads also have byte progress.

## Validation scope

Release gates cover Rust formatting, strict Clippy, all tests on Linux/Windows, real PDF rendering/Tesseract OCR, native UI captures, three CPU AI families, exact numbers, editable tables, figure evidence, writer references, actual installer AI setup, 0.3-to-0.4 upgrade, installed GUI/Word/font export and reinstall/uninstall retention. Synthetic smoke tests use a bounded 768-pixel input cap; production defaults are unchanged. `scripts/Smoke-AI.ps1 -FullQuality` runs the longer native-resolution checks.

GPU performance, broad natural handwriting and future publisher compatibility are not guaranteed by these synthetic checks. The installer is unsigned; release assets include checksums and build provenance.
