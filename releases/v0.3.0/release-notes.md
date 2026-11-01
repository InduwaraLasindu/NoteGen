## NoteGen 0.3.0 — Studio, Handwriting Profiles, TrueType Font Export & 3rd-Party AI

Install **NoteGen 0.3.0** directly on Windows 10/11 x64-compatible systems.

### Direct Downloads (Ready to Download & Install)
- **[NoteGen-Setup-0.3.0.exe](/api/release/v0.3.0/download/NoteGen-Setup-0.3.0.exe)** — Per-user Windows 10/11 x64 Installer Wizard (Start menu shortcuts, optional desktop shortcut, uninstaller, no administrator rights required).
- **[NoteGen-0.3.0.exe](/api/release/v0.3.0/download/NoteGen-0.3.0.exe)** — Standalone Windows x64 Portable Executable.
- **[NoteGen-Rust-Windows-x64.zip](/api/release/v0.3.0/download/NoteGen-Rust-Windows-x64.zip)** — Portable Windows x64 distribution archive.

### Highlighted Features in v0.3.0
- **NoteGen Studio (`Context & Writers`)**: Unified studio for document subject/glossary guidance, interactive clarification prompts, writer handwriting profiles, personal TrueType (`.ttf`) font pack export, 3rd-party AI model registration, and public Wikipedia/search grounding.
- **Interactive Clarification Inbox ("A little help with this page")**: Prompts for unrecognizable words, conflicting numbers/table cells, and diagram interpretations with zoomed source crops, one-click reader alternatives, and contextual AI re-reading.
- **Writer Handwriting Profiles & Personal TrueType (`.ttf`) Font Pack Export**: Learn individual writers' English letter/word patterns across pages, attach confirmed glyph reference sheets to vision models, average up to 32 samples per character, and export an installable `.ttf` font + SVG glyphs.
- **3rd-Party AI Models & SHA256 Publisher Updates**: Register local GGUF + projector pairs or loopback OpenAI-compatible vision servers (`LM Studio`, `Ollama`, `vLLM`), and check/apply SHA256-verified publisher updates with one-click rollback across `LightOnOCR-3`, `GLM-OCR`, `Qwen3.5`, and `Qwen3.8`.
- **Appearance & Motion Customization**: Configure dark/light surfaces, accent palette, motion speed, orbital particle density, and reduced-motion accessibility.

### Codebase Audit & Regression Fixes Included
- **NG-AUDIT-01** (`rust/src/writers.rs (normalize)`): False Dark-Background Inversion on Tight Handwritten Glyph Crops — Sample all 4 perimeter edges (top, bottom, left, right) with zero-division guard and require edge_mean < 96 before inverting.
- **NG-AUDIT-02** (`rust/src/consensus.rs (select)`): False-Positive Numeric Disagreement on Trailing Sentence Punctuation — Added .trim_matches(['.', ',']) after outer non-alphanumeric stripping so "1250." and "1250" normalize identically while preserving internal decimals like "48.5".
- **NG-AUDIT-03** (`rust/src/assistance.rs (questions)`): Stale Block Answer Prefix Suppressing Re-Read Clarification Prompts — Compute the full block digest ID first (p{n}-b{index}-{digest[..10]}) and match exact question IDs so modified blocks are re-prompted.
- **NG-AUDIT-04** (`rust/src/fontpack.rs (ttf)`): TrueType Binary Table Checksum & Unsorted Glyph Mapping Ordering — Enforced ascending Unicode codepoint sort before building glyf, loca, hmtx, and cmap Format 4 tables, and verified 0xB1B0AFBA head checkSumAdjustment.

### Chunked Verification Gate (6/6 Passed)
- **[PASSED] Chunk 1: Independent Family Medoid & Numeric Disagreement Gate** (35ms) — 4 independent families deduplicated · agreement 98.7% · trailing period normalized (1250, 48.5)
- **[PASSED] Chunk 2: Writer Profile Normalization & Multi-Sample Glyph Averaging** (1ms) — 50/62 alphanumeric coverage · 16×20 averaged 'r' grid verified · 4-border perimeter guard active
- **[PASSED] Chunk 3: TrueType (.ttf) Binary SFNT Table & 0xB1B0AFBA Checksum Gate** (16ms) — 50 glyphs compiled (18664 bytes) · SFNT checksum 0xB1B0AFBA
- **[PASSED] Chunk 4: Interactive Clarification & Block Digest Integrity Gate** (2ms) — 3 initial questions -> 1 resolved -> 2 remaining · FNV-1a/SHA256 digest guard verified
- **[PASSED] Chunk 5: 3rd-Party Reader Loopback Guard & SHA256 Revision Rollback** (1ms) — Remote endpoint rejected with NG203 · HTTP 206 Content-Range verified
- **[PASSED] Chunk 6: Word (.docx) OOXML Table Grid, Cell Merges & PKZIP Archive Gate** (2ms) — Valid PKZIP .docx (6409 bytes) · TableGrid & diagram context verified

### Documentation
- [Windows Installer Guide](https://github.com/InduwaraLasindu/NoteGen/blob/main/docs/INSTALLER.md)
- [NoteGen Studio Guide](https://github.com/InduwaraLasindu/NoteGen/blob/main/docs/STUDIO.md)
- [Testing & Verification Guide](https://github.com/InduwaraLasindu/NoteGen/blob/main/docs/TESTING.md)