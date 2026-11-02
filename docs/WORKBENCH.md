# Workbench

Open **Projects & tools** or press **Ctrl+K** to find an action. Import PDFs, PNGs, JPEGs or multipage 8-bit grayscale/RGB/RGBA TIFFs. Scan folders are read in filename order; use names such as `001.png`, `002.png`. Unsupported TIFF color depths are rejected with a conversion suggestion. Originals are preserved.

## Review

Compare the selected source region and all reader evidence. Save a draft or approval to create a named revision. Undo, redo, compare and restore operate on complete saved revisions, including cells, boxes, reading order and approvals. Save before navigating or applying another tool. Concurrent or external edits cause a conflict instead of silently replacing work.

The review queue ranks numeric disagreements, totals, table-column anomalies and uncertain regions. Its orange overlay marks affected source regions. These are suggestions, not automatic corrections or calibrated accuracy scores. Source zoom supports keyboard focus and the scrollable source pane.

Focus **Keyboard review** to use **Ctrl/Cmd+S** to save a draft, **Ctrl/Cmd+Enter** to approve, **Ctrl/Cmd+J** for the next suggestion and **Ctrl/Cmd+D** to defer the current suggestion. These shortcuts require the active review window and are disabled while editing text/JSON, running a conversion or using another tool. Optional auto-advance moves to the next page only after approval saves successfully. Deferrals and auto-advance last for the current review session; deferred issues remain visible and can be reset.

**Manual annotation layers** labels the selected recognized block as original text, highlight, marginal note, stamp, teacher correction or other annotation. Labels never remove source ink or recognized text. A changed block makes its label stale and requires explicit reconfirmation. Set, remove and reconfirm actions create undoable revisions. Exports retain labels and stale notices outside the editable source text. CLI equivalents are `layers`, `set-layer`, `remove-layer` and `reconfirm-layer`; CLI block indexes start at zero.

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
notegen-cli tools project-checks path/to/project numeric
notegen-cli tools project-checks path/to/project consistency
notegen-cli tools undo path/to/project
notegen-cli tools export path/to/project --format xlsx -o tables.xlsx
notegen-cli tools profile-coverage WRITER_ID
notegen-cli tools profile-exercises WRITER_ID
notegen-cli tools profile-snapshot WRITER_ID "Before new examples"
notegen-cli tools font-proof WRITER_ID -o proof.html
```

Use `--settings` before the command for a saved configuration. `tools score` reports character, word, exact-number and table-cell errors separately. `tools calibrate` reads only the calibration split and reports empirical bins with sample counts and intervals. No natural-handwriting corpus or universal accuracy score is bundled.

`convert ... --overlap-preparation` opts into bounded rendering/printed-OCR overlap. It requires multiple pages, at least two configured CPU cores and sufficient measured free RAM after reservations; otherwise conversion stays sequential. There is one OCR consumer and one queued page. AI begins after preparation workers join. Admission is a conservative estimate rather than an OS memory quota; the project's `preparation-report.json` records the decision.

The numeric check flags impossible ISO/slash dates, ambiguous date order, repeated signs and unusual separator patterns. It assumes no decimal locale and does not rewrite values. The consistency check links unit and labelled-identifier variants across pages. Intentional code, version numbers and distinct identifiers can explain a suggestion; compare the source.

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

`unwarp-scan photo.png curves.json -o flattened-copy` applies manual vertical curved-page correction. Supply output dimensions and 2–128 normalized samples for the top and bottom source curves, spanning `x=0` to `x=1`:

```json
{"width":1200,"height":1600,"knots":[{"x":0,"top":0.05,"bottom":0.95},{"x":0.5,"top":0.1,"bottom":0.9},{"x":1,"top":0.05,"bottom":0.95}]}
```

The new folder retains `original.png`, `corrected.png` and a hashed `relationship.json` with continuous coordinate mappings and pixel-change diagnostics. This flattens the supplied vertical geometry; it does not detect page shape or reconstruct a 3D surface. Cropped margins remain in the original. Inspect thin strokes and distorted regions before converting the corrected copy.

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

## Retention-managed sessions

For temporary work, use a dedicated new session under an existing parent folder. Keep is the default. Session conversion disables public lookup, clarification callbacks, writer examples and automatic update checks, and rejects endpoint readers.

```shell
notegen-cli tools session-create C:/PrivateWork
notegen-cli tools session-run SESSION_ROOT scan.pdf --preset fast
notegen-cli tools session-export SESSION_ROOT -o C:/Exports/notes.docx
notegen-cli tools session-inspect SESSION_ROOT -o retention-preview.json
```

Inspect the preview and put only the relative file paths you choose into a JSON array such as `selections.json`. `session-discard SESSION_ROOT retention-preview.json selections.json` requires the exact fresh preview and a verified external export. It removes only selected unchanged tracked files. Marker/state/locks, unknown or modified files, and empty folders remain; the cleanup report lists what was kept. `session-recover` handles interrupted staged cleanup. Failed/interrupted work remains untracked for manual inspection; `session-export --recovery` exports tracked artifacts to a new ZIP.

This controls NoteGen's recorded session artifacts. Native OCR/process temporary files, OS caches, pagefile and crash dumps are outside its control. It provides neither encryption nor secure erasure. Retained metadata includes paths, hashes and export receipts.

## Offline transfer, resources and diagnostics

Use `bundle-preview`, `export-bundle`, `inspect-bundle` and `import-bundle` for verified offline AI transfer. Import creates an isolated destination and saves a new settings file; it does not silently replace active registrations. A compatible installed NoteGen/native runtime is still required. `install-preflight` reports selected/reusable/download bytes, retained rollback space and conservative disk bounds.

The privacy workbench exposes reversible application resource profiles and Windows Credential Manager controls for this repository's private update access. Credentials are masked, excluded from saved settings/diagnostics and only retrieved for the fixed publisher API. Existing explicit environment-based authentication remains compatible. Application budgets are not OS power governors or hard memory quotas.

**Settings → Updates** defaults to the stable app channel. Preview is an explicit opt-in and shows publisher notes as bounded plain text. Neither channel downloads or installs automatically. Defer automatic publisher checks for 7 or 30 days, or clear the deferral; **Check all updates now** bypasses it. Channels apply to app releases, not model accuracy or a project's exact reader revision. Per-project version preferences are not yet exposed.

`hardware-trial` benchmarks a copied source page on explicitly selected compatible devices. Reports separate measured duration, sampled memory, review flags and optional groundtruth scorecards. VRAM can be unavailable; sampled highwater excludes startup spikes. A physical multi-GPU run is not implied by ordinary tests.

Support exports begin with `support-preview` and an explicit request file. The default includes only an allowlisted settings/build summary. Activity opt-in retains safe counters/codes rather than raw messages; document attachments require a separate content opt-in. Inspect exact files with `inspect-support`, then select files and supply the preview token to `export-support`. Repair and cleanup also require fresh, explicit selection previews. Cleanup targets only known verified runtime ZIP caches, protecting project/writer/model/rollback data.
