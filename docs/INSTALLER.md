# Install NoteGen 0.3.0 on Windows

Download **NoteGen-Setup-0.3.0.exe** from the [GitHub Releases page](https://github.com/InduwaraLasindu/NoteGen/releases/latest).
Run it and follow the wizard. The installer targets Windows 10 build 19041 or newer,
Windows 11, and x64-compatible systems. It installs for your Windows account without
requesting administrator rights. The wizard follows the Windows light/dark appearance.

The default folder is `%LOCALAPPDATA%\Programs\NoteGen`. Start menu shortcuts are created;
a desktop shortcut is optional. English printed OCR and editable Word export are included.
No Python, Rust toolchain, Office installation or developer tools are required to run it.

## What's New in NoteGen 0.3.0

- **NoteGen Studio (`Context & Writers`)**: Unified studio for document subject/glossary guidance, interactive clarification questions, writer handwriting profiles, personal TrueType (`.ttf`) font pack export, 3rd-party AI model registration, and public Wikipedia/search grounding.
- **Interactive Clarification Prompts ("A little help with this page")**: Automatically identifies low-confidence words, conflicting numbers/table cells, and diagram interpretations, showing the exact page crop and alternative reader candidates before Word export.
- **Handwriting Writer Profiles & TrueType (`.ttf`) Font Export**: Learn individual writers' English letter and word patterns across pages, send confirmed visual reference sheets to compatible vision models, average up to 32 glyph samples per character, and export an installable `.ttf` font pack + SVG glyphs.
- **3rd-Party AI Readers & Verified Publisher Updates**: Register local GGUF + projector pairs or loopback OpenAI-compatible vision servers (`LM Studio`, `Ollama`, `vLLM`), and check/apply SHA256-verified publisher updates with one-click rollback.
- **Appearance & Motion Customization**: Configure dark/light surfaces, accent palette, motion speed, orbital particle density, and reduced-motion accessibility.

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
reader set. See [Windows usage](WINDOWS.md) and [NoteGen Studio guide](STUDIO.md) for conversion, review, and handwriting font export.

## Upgrade or uninstall

Run the **NoteGen-Setup-0.3.0.exe** installer to upgrade an existing installation. Your selected CUDA/Vulkan
AI runtime, models, writer profiles, setup logs, settings and document projects are retained.
Uninstall from Windows **Settings → Apps → Installed apps** or the Start menu.

The installer is published after Windows install, dependency, GUI launch, Word export,
TrueType font validation, upgrade and uninstall tests pass. The release includes its SHA256
file and a provenance manifest identifying the verified Rust build.
