# 0.5.0 implementation ledger

This ledger covers all 100 proposals. Implemented describes the concrete code path; release validation is still in progress. Partial items explicitly retain missing scope. Planned items remain work, not shipped claims. Corpus collection (#1) and Windows signing (#10) were excluded at the user’s request. No release can promise universal accuracy or 100% stability.

| # | Proposal | Status | Implementation / remaining scope |
|---|---|---|---|
| 1 | Representative handwriting benchmark | Excluded | Corpus collection excluded by user. No real-handwriting accuracy claim. |
| 2 | Accuracy scorecards | Implemented | Separate character, word, exact-number and coordinate-addressed table-cell scorecards. |
| 3 | Calibrated uncertainty | Partial | Held-out calibration bins and intervals; no supplied real corpus or calibrated deployment thresholds. |
| 4 | Numeric integrity checks | Partial | Numeric disagreement/token and table-total suggestions; broader dates/units/identifier validation remains. |
| 5 | Reader selection by region | Partial | Explicit task/reader selection for region rereads. Measured automatic routing requires regional benchmark data. |
| 6 | Scan quality preflight | Partial | Blur, contrast, dimensions and possible blank/overexposed-page observations. General clipping/glare detection remains. |
| 7 | One-page trial run | Implemented | Desktop one-page trial using first selected page; separate output, timing and review result. |
| 8 | Source-to-output traceability | Implemented | Block/cell source/evidence manifest and source-linked text/Excel exports; localization precision is explicit. |
| 9 | Review undo and redo | Partial | Durable multi-step page/context undo/redo; learned profile examples use separate version/quarantine tools. |
| 10 | Signed Windows releases | Excluded | Windows Authenticode signing excluded by user; installer remains unsigned. |
| 11 | Automatic script detection | Partial | Recognized-Unicode script hints and manual OCR-language override; no image-based script classifier. |
| 12 | Mixed-language recognition | Planned | Implementation remains. |
| 13 | Multilingual resource packs | Planned | Implementation remains. |
| 14 | Right-to-left documents | Partial | Word RTL paragraphs/tables and mixed-direction text; corpus/native-language review remains. |
| 15 | Editable mathematics | Planned | Implementation remains. |
| 16 | Scientific notation tools | Partial | Source-linked technical-notation suggestions; no specialized chemistry recognizer. |
| 17 | Code transcription | Partial | Code kind and monospaced, whitespace-preserving Word/LaTeX output; handwriting code quality unmeasured. |
| 18 | Reading-order editor | Implemented | Move recognized blocks in review, retain source boxes, save/undo reading order. |
| 19 | Header and footer handling | Partial | Conservative repeated-edge proposals and selected undoable removal; Word-section mapping remains. |
| 20 | Footnotes and references | Partial | Citation/footnote candidates with ambiguity retained; complete semantic linking remains. |
| 21 | Shadow and glare recovery | Partial | Non-destructive shadow/uneven-light variants and ink-change warnings; destroyed glare detail cannot be recovered. |
| 22 | Curved-page correction | Planned | Implementation remains. |
| 23 | Perspective correction | Partial | Validated four-corner perspective transform via CLI; draggable desktop corner editor remains. |
| 24 | Handwritten line segmentation | Partial | Editable source bounds and page JSON; dedicated handwritten line-boundary editor remains. |
| 25 | Paper-background controls | Implemented | Optional pale ruling/grid suppression with original and comparison metrics. |
| 26 | Annotation layers | Planned | Implementation remains. |
| 27 | Redaction-aware recognition | Planned | Implementation remains. |
| 28 | Preprocessing comparison | Implemented | Cleanup variants, retained original and difference/ink-preservation reports. |
| 29 | Double-page splitting | Implemented | Double-page splitting with adjustable gutter, retained original and explicit source relationships. |
| 30 | Page organization | Partial | Ordered image-folder projects and reversible block order; general page insert/remove/split/merge editor remains. |
| 31 | Targeted region re-reading | Implemented | Selected reader/task/resolution/context region reread; source and confirmed edits retained. |
| 32 | Disagreement heatmap | Partial | Block-level disagreement/review overlay; word-level localization is unavailable when readers do not provide it. |
| 33 | Impact-based review queue | Implemented | Source-linked issues ranked by numeric/table/uncertainty impact. |
| 34 | Fast review controls | Partial | Keyboard-focusable review/palette actions; configurable auto-advance/defer workflow remains. |
| 35 | Revision timeline | Implemented | Named page/context revisions, snapshots, comparison and restore. |
| 36 | Context-change comparison | Implemented | Field-level revision differences retain uncertain interpretations and contextual evidence. |
| 37 | Interactive table-grid repair | Partial | Editable cells, headers, rows/columns, merge splitting and source bounds; visual grid-boundary repair remains. |
| 38 | Table consistency checks | Implemented | Conservative row/column-type and stated-total suggestions; no values rewritten. |
| 39 | Cross-page consistency review | Partial | Cross-page source-linked consistency suggestions; no forced normalization. |
| 40 | Reversible correction rules | Implemented | Literal rule preview, selected pages/block exclusions, stale-preview rejection and one-revision rollback. |
| 41 | Project dashboard | Implemented | Names, folders, tags, status, pending counts and recent-project dashboard. |
| 42 | Local full-text search | Implemented | Local recognized-text search with source page/block jumps; explicit project roots only. |
| 43 | Linked project references | Implemented | Local hashed references with explicit background roles separate from transcription evidence. |
| 44 | Project templates | Implemented | Validated portable project-policy templates with private/device fields excluded. |
| 45 | Queue priorities | Partial | Queue priorities and manual up/down ordering; urgent work starts between jobs, no stage preemption/dragging. |
| 46 | Content-based duplicate detection | Partial | Content-hash duplicate groups across projects; automated reuse/comparison import flow remains. |
| 47 | Watched import folders | Planned | Implementation remains. |
| 48 | Batch image capture | Implemented | Filename-ordered mixed image folders become one page-ordered project. |
| 49 | More input formats | Partial | PNG/JPEG and multipage 8-bit Gray/RGB/RGBA TIFF; other TIFF color/depth variants and HEIC are rejected. |
| 50 | Recovery dashboard | Implemented | Recovery report distinguishes completed checkpoints, corrections, missing sources/images and actions. |
| 51 | Example coverage coaching | Implemented | Active glyph/digit/condition coverage and concrete sample-coaching suggestions. |
| 52 | Writer-profile suggestions | Planned | Implementation remains. |
| 53 | Writer-specific evaluation | Partial | Versioned writer-separated held-out evaluation infrastructure; profile-specific real-data gain measurements remain. |
| 54 | Confusion-pair exercises | Implemented | Focused confusion-pair prompts from confirmed active examples. |
| 55 | Glossary suggestions | Implemented | Glossary suggestions from active confirmed labels; no automatic context changes. |
| 56 | Profile version comparisons | Implemented | Profile snapshots, comparisons and branching preserve sample images. |
| 57 | Bad-example quarantine | Implemented | Reasoned quarantine/restore excludes examples from active recognition, cache identity and fonts. |
| 58 | Selective profile sharing | Implemented | Selected-example sharing with explicit privacy options and token-bound preview. |
| 59 | Richer handwriting fonts | Planned | Implementation remains. |
| 60 | Font proof sheets | Implemented | Printable HTML proof sheets with embedded real TTF and missing-glyph warnings. |
| 61 | Document-based hardware trials | Partial | One-page trial records time and quality/review separately; comparative document hardware trials remain. |
| 62 | Multi-GPU benchmarking | Planned | Implementation remains. |
| 63 | Measured memory budgets | Planned | Implementation remains. |
| 64 | Power and thermal profiles | Implemented | Quiet/battery/balanced/throughput application-budget previews with apply/undo and clear limits. |
| 65 | Resource sharing limits | Partial | Inference/OCR worker limits and memory admission reservations; not hard OS quotas. |
| 66 | Bounded warm-reader reuse | Planned | Implementation remains. |
| 67 | ETA confidence ranges | Implemented | Planning window from stage timing variability, sparse-data reasons and queue extrapolation; not calibrated confidence. |
| 68 | Time-budget planning | Implemented | Ordered time-budget forecasts with measured/estimated/unknown durations explicit. |
| 69 | Overlapped preparation | Planned | Implementation remains. |
| 70 | Fine-grained reconversion | Partial | Selected page/context/region rereads preserve unaffected output; automatic dependency-based invalidation remains. |
| 71 | Exact installer preflight | Partial | Selected bytes, verified reuse, remaining downloads, retained rollback and disk estimates; runtime expansion is a conservative upper bound. |
| 72 | Model capability cards | Partial | Capability cards expose task protocols, sources, revisions and estimates; missing license/language measurements are explicitly unknown. |
| 73 | Download scheduling | Planned | Implementation remains. |
| 74 | Offline AI bundles | Implemented | Verified offline model/runtime bundles, inventory preview and isolated import without changing active settings. |
| 75 | Shared model storage | Partial | Offline bundle objects deduplicate by hash; shared global version-aware weight storage remains. |
| 76 | Canary update validation | Planned | Implementation remains. |
| 77 | Per-project model pinning | Partial | Project fingerprints bind input/settings/models and reject mismatches; revision resolution/restoration UI remains. |
| 78 | Update channels | Planned | Implementation remains. |
| 79 | Safe storage cleanup | Planned | Implementation remains. |
| 80 | Private-update credentials | Planned | Implementation remains. |
| 81 | Privacy dashboard | Partial | Privacy panel reports configured connections, update/lookup state and data locations; not OS connection monitoring. |
| 82 | Encrypted workspaces | Planned | Implementation remains. |
| 83 | Confidential session mode | Planned | Implementation remains. |
| 84 | Share-ready redaction | Partial | Opaque image-copy redaction of explicit regions; recognized text/project metadata redaction remains. |
| 85 | Inspectable diagnostic bundles | Implemented | Inspectable selected diagnostics; content excluded by default, safe allowlist summaries and token-bound export. |
| 86 | Plain-language troubleshooting | Implemented | Plain-language error checks and recovery actions in diagnostics plus existing NG remedies. |
| 87 | Verified repair tools | Partial | Selective verified repair of compiled-pinned assets without registry activation; extracted runtime repair uses installer. |
| 88 | Migration recovery | Partial | Backward-compatible defaults plus transactional review/context recovery; general schema migration framework remains. |
| 89 | Failure-injection testing | Partial | Automated journal power-loss, collisions, cancellation, tampered input and simulated disk-full tests; real GPU/storage disconnect coverage remains. |
| 90 | Stronger build provenance | Partial | Locked CycloneDX inventory, binary/source/input hashes and provenance script; independent reproducibility/attestations remain. |
| 91 | Command palette and shortcuts | Partial | Searchable command palette and visible keyboard focus; fully configurable shortcuts remain. |
| 92 | Screen-reader support | Planned | Native accessibility integration and Narrator audit remain. |
| 93 | Readable-layout presets | Partial | Large/spacious/compact/high-contrast presets; full clipping audit at each viewport remains. |
| 94 | Precision source magnifier | Partial | Keyboard-accessible source zoom, source bounds and disagreement markers; full-resolution synchronized magnifier remains. |
| 95 | Interface localization | Planned | Implementation remains. |
| 96 | Semantic Word styling | Implemented | Configurable semantic Word styles for headings/lists/captions/equations/references/code. |
| 97 | Additional text exports | Implemented | Markdown/HTML/LaTeX with escaping, source references and uncertainty. |
| 98 | Searchable PDF export | Planned | Native source-preserving searchable PDF implementation and tests in progress. |
| 99 | Spreadsheet table export | Implemented | Real XLSX cells with literal values, source links, review sheet and merge warnings. |
| 100 | Word correction round-trip | Implemented | Tagged Word correction preview/import, conflict detection and one-revision selected apply. |
