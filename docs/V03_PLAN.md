# NoteGen 0.3 implementation gates

1. Audit and regressions: preserve manual edits; bounded parsing; cancellation;
   memory recovery; progress and resume identity. Gate: focused Rust tests, then baseline suite.
2. Model management: custom local GGUF readers and local OpenAI-compatible vision servers;
   explicit model updates, SHA256 verification, version history and rollback. No remote
   document inference by default and no execution of downloaded model repository code.
3. Writer memory: named local profiles; confirmed word/letter crops, visual prototypes,
   correction history and a reference sheet for compatible vision readers. This is local
   example-based adaptation, not neural weight fine-tuning. Profiles are removable/exportable.
4. Clarification: optional document context, region notes and literal corrections;
   questions for uncertain text/numbers/figures before final export; skip/cancel paths;
   user-controlled public fact lookup with source citations. Web content never becomes OCR truth.
5. Font packs: average explicitly labelled glyph samples, trace contours and export a real
   installable TrueType font plus SVG glyphs, coverage report and preview. Missing glyphs
   are reported; letters are not invented from unsegmented cursive paragraphs.
6. Native UX: writer/context tools, model manager, clarification inbox, crop selection,
   theme presets and refined motion. Gate: UI state tests and native screen inspection.
7. Full Windows/Linux suites, actual CPU model conversion, font loading, installed-app
   conversion/upgrade/uninstall tests, then publish a new EXE installer release.

Tests are run in focused batches before integration. Target-PC RTX memory/latency and
broad handwriting/corpus accuracy remain separate acceptance measurements.

## Local gate result (2026-11-01)

81 ordinary Rust tests passed with all features/targets, including native PDFium/Tesseract
conversion. Format and Clippy (-D warnings) passed. Focused gates covered profile memory,
TTF outlines/checksums/mappings, font/profile packs, clarification data protection,
loopback third-party payloads with visual examples, update SHA/range checks, and Studio
rendering. Independent fontTools and FreeType readers parsed/rendered the exported font.
Windows/Linux CI, actual native model inference and installer acceptance follow this gate.
