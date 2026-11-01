# Install NoteGen on Windows

Download the latest **NoteGen-Setup-*.exe** from the [GitHub Releases page](https://github.com/InduwaraLasindu/NoteGen/releases/latest).
Run it and follow the wizard. The installer targets Windows 10 build 19041 or newer,
Windows 11, and x64-compatible systems. It installs for your Windows account without
requesting administrator rights. The wizard follows the Windows light/dark appearance.

The default folder is `%LOCALAPPDATA%\Programs\NoteGen`. Start menu shortcuts are created;
a desktop shortcut is optional. English printed OCR and editable Word export are included.
No Python, Rust toolchain, Office installation or developer tools are required to run it.

## Set up local AI

Leave **Install local AI and all required models for this PC** selected in the installer. The wizard detects hardware, downloads a compatible native runtime and all fitting reader families needed for Maximum Accuracy, verifies checksums, and activates them. It shows progress, supports cancellation and resumes partial downloads. Uncheck the option for an offline printed-OCR installation. AI models can require several GB.

Inside NoteGen, **Settings > Updates & installation > Install all required AI for this PC** performs the same setup. CUDA 13/12, Vulkan and permitted CPU fallback are probed as needed. Models and documents stay local. Healthy installed models and newer runtime versions are reused.

The same Settings window offers publisher update checks, verified model/runtime updates, app updates for bundled dependencies, and runtime rollback. **Hardware test** measures CPU workers and up to two synthetic AI readings; review and apply its recommendation. **Everyday settings** has descriptions and a live-preview option; **Setting descriptions** covers the full Advanced configuration.

Silent installs do not download AI unless `/AI=1` is explicitly supplied. See [0.4.0 release details](RELEASE_0.4.0.md) for deployment flags, cancellation, update authentication and validation scope.

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
