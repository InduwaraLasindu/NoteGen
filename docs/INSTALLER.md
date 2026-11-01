# Install NoteGen on Windows

Download **NoteGen-Setup-0.2.0.exe** from the [GitHub Releases page](https://github.com/InduwaraLasindu/NoteGen/releases/latest).
Run it and follow the wizard. The installer targets Windows 10 build 19041 or newer,
Windows 11, and x64-compatible systems. It installs for your Windows account without
requesting administrator rights. The wizard follows the Windows light/dark appearance.

The default folder is `%LOCALAPPDATA%\Programs\NoteGen`. Start menu shortcuts are created;
a desktop shortcut is optional. English printed OCR and editable Word export are included.
No Python, Rust toolchain, Office installation or developer tools are required to run it.

## Set up local AI

Choose **NoteGen → Set up local AI** in the Start menu. It downloads the pinned native
runtime and models selected for your available RAM/VRAM. NVIDIA PCs select CUDA 13;
the setup console records progress and SHA256 checks. Model downloads can require
several GB. Restart NoteGen after setup. Conversion never uploads documents.

For an older driver, open PowerShell in the installation folder and run:

```powershell
.\scripts\Configure-AI.ps1 -Backend CUDA12
```

Other choices are `Auto`, `Vulkan` and `CPU`. Add `-Balanced` for the smaller initial
reader set. Runtime/model compatibility and actual RTX performance still require
validation on the target PC. See [Windows usage](WINDOWS.md) for conversion and review.

## Upgrade or uninstall

Run a newer NoteGen installer to upgrade the same installation. Your selected CUDA/Vulkan
AI runtime is retained; run AI setup again when you want to update it. Uninstall from Windows
**Settings → Apps → Installed apps** or the Start menu. Models, setup logs, settings and
document projects are retained. Back up your projects before manually deleting data.

The installer is unsigned. It is published only after real Windows install, dependency,
GUI launch, Word export, upgrade and uninstall tests pass. The release includes its SHA256
file and a manifest identifying the original verified Rust build. The base app passed
65 Rust tests on Windows/Linux and real CPU AI conversion tests; installer packaging
does not change its executable code.

## Reproduce the installer

The `Windows installer and GitHub Release` workflow downloads the immutable successful
Rust build, verifies provenance/checksums, bundles the app-local Microsoft CRT, and uses
the official SHA256-pinned Inno Setup 7.1.1 compiler. It tests installation in a path
containing spaces and checks preservation of user-created files before publishing.
The AI setup dry run performs no downloads. This pipeline does not repeat unaffected
Rust/model tests; those are attached to the verified source build.
