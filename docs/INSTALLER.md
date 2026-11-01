# Install NoteGen on Windows

Download the latest **NoteGen-Setup-*.exe** from the [GitHub Releases page](https://github.com/InduwaraLasindu/NoteGen/releases/latest).
Run it and follow the wizard. The installer targets Windows 10 build 19041 or newer,
Windows 11, and x64-compatible systems. It installs for your Windows account without
requesting administrator rights. The wizard follows the Windows light/dark appearance.

The default folder is `%LOCALAPPDATA%\Programs\NoteGen`. Start menu shortcuts are created;
a desktop shortcut is optional. English printed OCR and editable Word export are included.
No Python, Rust toolchain, Office installation or developer tools are required to run it.

## Set up local AI

Click **Download & install AI** inside NoteGen. The card lists the recommended
readers and model download size. It uses the current preset (Fast installs the Accuracy
reader set) and available memory. Setup downloads a native Windows runtime and matching
model/projector pairs, verifies SHA256, probes runtime/GPU access and registers the readers.
You can start converting without restarting. CUDA 13/12, Vulkan and permitted CPU fallback
are tried as needed; Advanced → Hardware lets you select a backend or require acceleration.

Downloads can require several GB, plus runtime files. The overall and byte progress bars
remain visible. Cancel preserves existing active versions and resumable partial downloads.
Retrying reuses checksum-verified models, including newer publisher revisions.
Conversion does not upload documents. Studio → Readers offers custom readers, explicit
publisher update checks and rollback.

The Start menu **Set up local AI** helper remains a manual alternative. CLI automation:

```powershell
.\notegen-cli.exe setup --preset maximum --backend auto -o ai-settings.json
.\notegen-cli.exe --settings ai-settings.json convert notes.pdf -o notes.docx
```

Use `--runtime-only` or `--models-only` for individual stages, or
`--model-ids glm-ocr,lighton3-1b,qwen35-2b` for an explicit verified reader set.
Actual RTX performance and broad handwriting accuracy still need measurement on the
user's PC. See [Windows usage](WINDOWS.md) for conversion and review.

## Upgrade or uninstall

Run a newer NoteGen installer to upgrade the same installation. Your selected CUDA/Vulkan
AI runtime is retained; run AI setup again when you want to update it. Uninstall from Windows
**Settings → Apps → Installed apps** or the Start menu. Models, setup logs, settings and
document projects are retained. Back up your projects before manually deleting data.

The installer is unsigned. It is published only after real Windows install, dependency,
GUI launch, Word export, upgrade and uninstall tests pass. The release includes its SHA256
file and a manifest identifying the original verified Rust build. The source workflow runs the full Windows/Linux Rust suites, native desktop checks and
real CPU AI conversion. Installer packaging uses those exact executable bytes.

## Reproduce the installer

The `Windows installer and GitHub Release` workflow downloads the immutable successful
Rust build, verifies provenance/checksums, bundles the app-local Microsoft CRT, and uses
the official SHA256-pinned Inno Setup 7.1.1 compiler. It tests installation in a path
containing spaces and checks preservation of user-created files before publishing.
The AI setup dry run performs no downloads. This pipeline does not repeat unaffected
Rust/model tests; those are attached to the verified source build.
