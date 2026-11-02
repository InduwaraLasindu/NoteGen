# Workbench

Open **Projects & tools** or press **Ctrl+K** to find an action. Import PDFs, PNGs, JPEGs or multipage 8-bit grayscale/RGB/RGBA TIFFs. Scan folders are read in filename order; use names such as `001.png`, `002.png`. Unsupported TIFF color depths are rejected with a conversion suggestion. Originals are preserved.

## Review

Compare the selected source region and all reader evidence. Save a draft or approval to create a named revision. Undo, redo, compare and restore operate on complete saved revisions, including cells, boxes, reading order and approvals. Save before navigating or applying another tool. Concurrent or external edits cause a conflict instead of silently replacing work.

The review queue ranks numeric disagreements, totals, table-column anomalies and uncertain regions. Its orange overlay marks affected source regions. These are suggestions, not automatic corrections or calibrated accuracy scores. Source zoom supports keyboard focus and the scrollable source pane.

The export/rules tab previews a literal replacement across the project. Applying the inspected preview creates one revision; undo restores the previous values. CLI rules accept selected stable page numbers and block exclusions.

## Projects

The project dashboard reads the recent projects you selected, with names, folders, tags, status and pending-review counts. Full-text search includes recognized cells and diagram labels; selecting a hit opens the matching page and region. Duplicate detection compares input hashes. Recovery reports distinguish missing sources, checkpoints, images and pending review.

Project details are editable, validated JSON. Attach local references as background, diagram or reference files; they do not become transcription evidence. Templates retain recognition/context/export policy while excluding private device paths, writer identity and public-source excerpts.

## Exports

Word uses semantic styles, source controls and a correction baseline. Markdown, HTML and LaTeX retain source references and uncertainty annotations. Excel stores recognized cell values as literal text, including leading zeros and formula-like strings, and includes a review sheet. Traceability JSON records every block and cell, candidate evidence and the actual available source region. Individual cell coordinates are not invented when only a table box exists.

All exporters protect existing filenames. Source links refer to files in the project; keep the project available when following them. Sharing an export and its referenced source is a separate, explicit action.

## CLI

```shell
notegen-cli tools --help
notegen-cli tools preflight scan.png --page 1
notegen-cli convert scan-folder -o notes.docx --preset fast
notegen-cli tools search "total" path/to/project
notegen-cli tools recovery path/to/project
notegen-cli tools timeline path/to/project
notegen-cli tools undo path/to/project
notegen-cli tools export path/to/project --format xlsx -o tables.xlsx
notegen-cli tools profile-coverage WRITER_ID
notegen-cli tools profile-exercises WRITER_ID
notegen-cli tools profile-snapshot WRITER_ID "Before new examples"
notegen-cli tools font-proof WRITER_ID -o proof.html
```

Use `--settings` before the command for a saved configuration. `tools score` reports character, word, exact-number and table-cell errors separately. `tools calibrate` reads only the calibration split and reports empirical bins with sample counts and intervals. No natural-handwriting corpus or universal accuracy score is bundled.

Profile sharing requires an inspected selection preview token. Quarantined examples retain a reason and can be restored; they are excluded from active recognition and generated glyphs. Font proof sheets include missing-glyph warnings. They do not imply complete cursive shaping or additional-script support.

## Language packs and interface

Core interface labels offer English, Sinhala and Tamil in **Settings**. Detailed help remains English. This selection does not change recognition languages.

```shell
notegen-cli tools language-packs
notegen-cli tools language-plan ocr-resources/tessdata eng sin tam
notegen-cli tools install-languages ocr-resources/tessdata eng sin tam -o multilingual-settings.json
notegen-cli --settings multilingual-settings.json convert scan.pdf -o notes.docx --preset fast
```

The optional printed-OCR packs come from an immutable official Tesseract revision. SHA256, byte lengths and the Apache license are verified. English and orientation data accompany the selection. Language order is manual (`eng+sin+tam`); automatic paragraph routing and broad Sinhala/Tamil recognition accuracy are not established by installation.

## Region tools and scan preparation

`tools region-read PROJECT REQUEST.json` accepts a selected page, a normalized source rectangle, an installed reader ID, image edge (0 for automatic), task and context. It saves evidence as an undoable revision and retains confirmed text. Tasks include text, handwriting, table, equation, figure and code.

```json
{"page":1,"region":{"left":0.1,"top":0.2,"right":0.8,"bottom":0.5},"reader":"glm-ocr","image_edge":1536,"task":"table","context":"Check the original decimal signs."}
```

`cleanup-scan` writes the original and cleanup variants to a new folder. `perspective-scan` reads four normalized points in top-left, top-right, bottom-right, bottom-left order. `split-scan` retains the original and gutter relationships. `redact-scan` writes an opaque PNG copy for explicit rectangles; it does not redact text exports or project metadata automatically. Each tool has `--help` and protects existing outputs.

## Word corrections and mathematics

Export Word from the current project, edit its source-tagged fields, then select **Inspect corrected Word document** in the export workbench. Select the changed fields to import. Missing controls, changed structure, stale previews and simultaneous project edits become conflicts. Importing creates one undoable revision; source recognition remains intact.

Editable Office Math supports groups, fractions, subscripts/superscripts, roots, common Greek/operators, text runs and basic matrices. Unsupported LaTeX remains visible as literal source with a review caption. This is a bounded grammar, not a general TeX engine. Word style profiles are available with `tools word-styles`.

## Watched folders

The CLI watcher runs until cancelled. Its configuration is reloaded while running. Keep input, output and state locations distinct. Set `paused` in the configuration, or create the `--pause-file`, to stop new imports. Global `--cancel-file` also stops an active conversion.

```json
{"folder":"C:/Scans/Inbox","extensions":["pdf","png","jpg","tiff"],"stable_seconds":3,"paused":false,"duplicate_policy":"skip","max_file_bytes":1000000000}
```

```shell
notegen-cli --cancel-file stop-watch tools watch watch.json watch-state.json output --pause-file pause-watch
```

Files must remain stable across polls. Only successful conversions receive an import receipt; failed files retain checkpoints and use bounded retry backoff. `new_revision` permits separate identical-content imports. `--once` processes only files already stable during that poll.

## Offline transfer, resources and diagnostics

Use `bundle-preview`, `export-bundle`, `inspect-bundle` and `import-bundle` for verified offline AI transfer. Import creates an isolated destination and saves a new settings file; it does not silently replace active registrations. A compatible installed NoteGen/native runtime is still required. `install-preflight` reports selected/reusable/download bytes, retained rollback space and conservative disk bounds.

The privacy workbench exposes reversible application resource profiles and Windows Credential Manager controls for this repository's private update access. Credentials are masked, excluded from saved settings/diagnostics and only retrieved for the fixed publisher API. Existing explicit environment-based authentication remains compatible. Application budgets are not OS power governors or hard memory quotas.

`hardware-trial` benchmarks a copied source page on explicitly selected compatible devices. Reports separate measured duration, sampled memory, review flags and optional groundtruth scorecards. VRAM can be unavailable; sampled highwater excludes startup spikes. A physical multi-GPU run is not implied by ordinary tests.

Support exports begin with `support-preview` and an explicit request file. The default includes only an allowlisted settings/build summary. Activity opt-in retains safe counters/codes rather than raw messages; document attachments require a separate content opt-in. Inspect exact files with `inspect-support`, then select files and supply the preview token to `export-support`. Repair and cleanup also require fresh, explicit selection previews. Cleanup targets only known verified runtime ZIP caches, protecting project/writer/model/rollback data.
