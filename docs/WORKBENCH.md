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
