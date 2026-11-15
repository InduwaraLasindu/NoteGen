# 0.5.0 implementation ledger

The 0.5.0 scope is frozen at the user's request: finish and validate the current work, then publish. This ledger maps the original 100 proposals to the resulting code; it does not claim all 100 are complete. **Implemented** means the production path exists, not that every hardware, language or release acceptance gate has passed. **Partial** records missing code or external acceptance, deferred beyond this release. Corpus collection (#1) and Windows signing (#10) remain excluded by request.

The [verification record](TESTING.md) identifies the tested application source, `19550373dd25afb36b21a37add0172b46da84117`. Its Windows/Linux functional and native checks passed. Independent executable comparison failed and is deferred for 0.5.0 under the user's instruction to skip the blocked GitHub testing path. Later build-metadata changes on main are not included in these application binaries. No universal accuracy or 100% stability claim is made.

Current mapping: **86 implemented, 12 partial, 2 excluded**. Partial work is recorded for future releases and does not reopen the frozen 0.5.0 scope.

| # | Proposal | Status | Implementation / remaining scope |
|---|---|---|---|
| 1 | Representative handwriting benchmark | Excluded | Corpus collection excluded by user. No real-handwriting accuracy claim. |
| 2 | Accuracy scorecards | Implemented | Separate character, word, exact-number and coordinate-addressed table-cell scorecards. |
| 3 | Calibrated uncertainty | Partial | Held-out calibration bins with sample counts and intervals; deployable hash-bound calibration and abstention policies are deferred. No real-handwriting calibration claim. |
| 4 | Numeric integrity checks | Implemented | Numeric disagreement, table totals, calendar/date-order, repeated-sign, separator, cross-page unit and labelled-identifier observations; ASCII/locale limits explicit, no automatic rewriting. |
| 5 | Reader selection by region | Partial | Explicit task/reader/resolution selection for region and repaired-line rereads. Learned regional routing artifacts and their held-out validation are deferred. |
| 6 | Scan quality preflight | Implemented | Bounded blur, contrast, boundary-ink/clipping, tiny-component and possible localized glare observations with rescan suggestions; these are heuristics, not diagnoses or recovered detail. |
| 7 | One-page trial run | Implemented | Desktop one-page trial using first selected page; separate output, timing and review result. |
| 8 | Source-to-output traceability | Implemented | Block/cell source/evidence manifest and source-linked text/Excel exports; localization precision is explicit. |
| 9 | Review undo and redo | Implemented | Durable page/context revisions cover text, cells, regions and approvals; writer-example changes have their own reversible history with retained sample images. |
| 10 | Signed Windows releases | Excluded | Windows Authenticode signing excluded by user; installer remains unsigned. |
| 11 | Automatic script detection | Implemented | Image-based native OSD suggestions with source binding, native confidence and explicit unavailable state; users confirm/correct languages, with no automatic setting change or calibrated language probability. |
| 12 | Mixed-language recognition | Partial | Combined verified OCR languages and source-bound paragraph/line routes with preview and confirmation; automatic language-region classification remains deferred. |
| 13 | Multilingual resource packs | Implemented | Optional SHA256-pinned official English/Sinhala/Tamil/Arabic/Hebrew printed OCR packs with exact plan and install/verify. |
| 14 | Right-to-left documents | Implemented | Word RTL paragraphs, mixed-direction runs and explicit physical table-column handling; confirmed text stays literal. Native-language visual acceptance remains external. |
| 15 | Editable mathematics | Implemented | Editable OMML equations for a bounded LaTeX grammar; unsupported source stays literal with explicit review warnings. |
| 16 | Scientific notation tools | Implemented | Bounded chemical formula/reaction parsing checks element counts and charge alongside source-linked technical notation observations; no silent balancing or replacement, and unsupported grammar is explicit. |
| 17 | Code transcription | Implemented | Literal code mode, native TXT/TSV evidence, optional confirmed indentation/gutter calibration, disagreement-preserving consensus and monospaced exports; no execution or syntax repair. Handwritten code accuracy is unmeasured. |
| 18 | Reading-order editor | Implemented | Move recognized blocks in review, retain source boxes, save/undo reading order. |
| 19 | Header and footer handling | Implemented | Conservative recurring-edge proposals with explicit keep/remove and undo. Automatic Word-section mapping is not included. |
| 20 | Footnotes and references | Implemented | Token-confirmed, source-bound footnote/citation/bibliography relationships, real Word footnotes and linked exports; ambiguous or stale links retain original text and warnings rather than invent matches. |
| 21 | Shadow and glare recovery | Implemented | Non-destructive shadow/uneven-light cleanup variants with original comparison and ink-change warnings; destroyed glare detail cannot be recovered. |
| 22 | Curved-page correction | Implemented | User-supplied bounded top/bottom curves flatten vertical page geometry via CLI; original, hashed mapping, change metrics and distortion warnings retained. Automatic/3D surface reconstruction is not provided. |
| 23 | Perspective correction | Implemented | Editable, validated four-corner perspective transform with retained original and geometric/change warnings. |
| 24 | Handwritten line segmentation | Implemented | Confirmed ordered line quadrilaterals, source overlay and undo; selected-line rereads whiten pixels outside the repaired boundary and preserve masks and reader evidence. No automatic handwriting line accuracy claim. |
| 25 | Paper-background controls | Implemented | Optional pale ruling/grid suppression with original and comparison metrics. |
| 26 | Annotation layers | Implemented | Human-confirmed block labels for original/highlight/margin/stamp/correction layers, exact source bindings, visible staleness, undo and export provenance; all text retained, no automatic ink separation. |
| 27 | Redaction-aware recognition | Implemented | No-recognition preparation, human-confirmed source-hash-bound covers and opaque-cover proposals; every reader/crop is masked, sensitive context omitted and ungrounded/intersecting output withheld. Stale source fails closed; original data/history remain private. |
| 28 | Preprocessing comparison | Implemented | Cleanup variants, retained original and difference/ink-preservation reports. |
| 29 | Double-page splitting | Implemented | Double-page splitting with adjustable gutter, retained original and explicit source relationships. |
| 30 | Page organization | Implemented | Previewed reorder/insert/archive/split/merge with stable source IDs, exact crop/composition mappings, durable undo/redo and original evidence/history/locks retained. Crossing or ungrounded derived text is withheld for review. |
| 31 | Targeted region re-reading | Implemented | Selected reader/task/resolution/context region reread; source and confirmed edits retained. |
| 32 | Disagreement heatmap | Implemented | Bounded word/cell overlays use supplied coordinates and independent reader families; missing precision retains coarse block issues. Coordinates are never inferred from string length. |
| 33 | Impact-based review queue | Implemented | Source-linked issues ranked by numeric/table/uncertainty impact. |
| 34 | Fast review controls | Implemented | Explicit keyboard-review focus, configurable save/approve/next/defer shortcuts, visible session deferrals and optional advance only after durable approval; source/editor panes scroll while essential actions remain reachable. |
| 35 | Revision timeline | Implemented | Named page/context revisions, snapshots, comparison and restore. |
| 36 | Context-change comparison | Implemented | Field-level revision differences retain uncertain interpretations and contextual evidence. |
| 37 | Interactive table-grid repair | Implemented | Source-overlaid row/column boundary proposals and explicit old-to-new cell mappings with preview/undo; split cells start empty and merges cannot hide nonempty values. |
| 38 | Table consistency checks | Implemented | Conservative row/column-type and stated-total suggestions; no values rewritten. |
| 39 | Cross-page consistency review | Implemented | Source-linked cross-page name, abbreviation, unit and repeated-value observations; user review controls every correction. |
| 40 | Reversible correction rules | Implemented | Literal rule preview, selected pages/block exclusions, stale-preview rejection and one-revision rollback. |
| 41 | Project dashboard | Implemented | Names, folders, tags, status, pending counts and recent-project dashboard. |
| 42 | Local full-text search | Implemented | Local recognized-text search with source page/block jumps; explicit project roots only. |
| 43 | Linked project references | Implemented | Local hashed references with explicit background roles separate from transcription evidence. |
| 44 | Project templates | Implemented | Validated portable project-policy templates with private/device fields excluded. |
| 45 | Queue priorities | Implemented | Priority levels and manual up/down order; urgent inputs yield lower-priority work after durable preparation/reader/finalization checkpoints, then resume original settings/output. Active native requests finish before yielding. |
| 46 | Content-based duplicate detection | Implemented | Explicit exact-compatible reuse, read-only comparison or private separate revision; input/configuration/model/runtime/OCR hashes are bound to preview. Masked comparisons withhold both sides' text; lineage and unknown private files survive forks. |
| 47 | Watched import folders | Implemented | Sequential watched-folder CLI worker with stable files, explicit rules, pause/cancel, duplicate policy, success receipts and retry backoff. |
| 48 | Batch image capture | Implemented | Filename-ordered mixed image folders become one page-ordered project. |
| 49 | More input formats | Implemented | PNG/JPEG and multipage 8-bit Gray/RGB/RGBA TIFF; unsupported TIFF color/depth variants and HEIC are explicitly rejected rather than guessed. |
| 50 | Recovery dashboard | Implemented | Recovery report distinguishes completed checkpoints, corrections, missing sources/images and actions. |
| 51 | Example coverage coaching | Implemented | Active glyph/digit/condition coverage and concrete sample-coaching suggestions. |
| 52 | Writer-profile suggestions | Implemented | Opt-in known-glyph profile suggestions; uncalibrated similarity and explicit token-bound confirmation, no automatic writer selection. |
| 53 | Writer-specific evaluation | Partial | Writer-separated held-out evaluation infrastructure; paired profile/baseline evaluation workflow and measured personalization gains are deferred. |
| 54 | Confusion-pair exercises | Implemented | Focused confusion-pair prompts from confirmed active examples. |
| 55 | Glossary suggestions | Implemented | Glossary suggestions from active confirmed labels; no automatic context changes. |
| 56 | Profile version comparisons | Implemented | Profile snapshots, comparisons and branching preserve sample images. |
| 57 | Bad-example quarantine | Implemented | Reasoned quarantine/restore excludes examples from active recognition, cache identity and fonts. |
| 58 | Selective profile sharing | Implemented | Selected-example sharing with explicit privacy options and token-bound preview. |
| 59 | Richer handwriting fonts | Implemented | Confirmed Unicode scalar, complete cluster/ligature and alternate shapes generate supplementary cmap/GSUB with shaping regressions and proof sheets. Missing forms use explicit placeholders; contextual joining/positioning is not invented. Native-language visual acceptance remains. |
| 60 | Font proof sheets | Implemented | Printable HTML proof sheets with embedded real TTF and missing-glyph warnings. |
| 61 | Document-based hardware trials | Implemented | Opt-in same-page copied-source hardware trials; measured times separated from optional groundtruth scores. |
| 62 | Multi-GPU benchmarking | Partial | Comparable trials on actually discovered compatible devices; target multi-GPU measurements remain unperformed. |
| 63 | Measured memory budgets | Partial | Sampled process RSS/NVIDIA VRAM highwater with unavailable/startup gaps explicit. True instantaneous peaks and learned future admission are deferred; current memory admission remains conservative estimates. |
| 64 | Power and thermal profiles | Implemented | Quiet/battery/balanced/throughput application-budget previews with apply/undo and clear limits. |
| 65 | Resource sharing limits | Implemented | Inference/OCR CPU budgets and memory admission reservations preserve configured application capacity; these are not hard OS quotas. |
| 66 | Bounded warm-reader reuse | Implemented | Opt-in app/watch pool holds at most one owned compatible reader for 1–60 seconds, verifies fresh context, rechecks admission and exposes immediate release; cancellation/errors release it. Private sessions, masked work and trials stay cold. Native RAM may retain transient buffers until release. |
| 67 | ETA confidence ranges | Implemented | Planning window from stage timing variability, sparse-data reasons and queue extrapolation; not calibrated confidence. |
| 68 | Time-budget planning | Implemented | Ordered time-budget forecasts with measured/estimated/unknown durations explicit. |
| 69 | Overlapped preparation | Implemented | Explicit CLI opt-in overlaps caller-thread rendering with one OCR consumer and one queued page; conservative CPU/RAM admission, sequential fallback, joined cancellation, AI starts afterwards. |
| 70 | Fine-grained reconversion | Implemented | Selected page/context/region/line rereads preserve unaffected output, source evidence and confirmed edits; no speculative dependency rewriting. |
| 71 | Exact installer preflight | Implemented | Selected readers, verified reuse, remaining transfer bytes, rollback retention and safe disk estimates; unknown runtime expansion is explicitly a conservative upper bound. |
| 72 | Model capability cards | Partial | Offline cards bind publisher/converter/license sources, immutable document hashes, exact asset pins, protocols and limitations. Memory figures are estimates and tested-language measurements remain explicitly unknown. |
| 73 | Download scheduling | Implemented | Persisted transfer manager with bandwidth limits, metered/unknown-network policy, UTC windows, per-job overrides, pause/resume/cancel and verified Range recovery; credentials and signed URLs are not persisted. |
| 74 | Offline AI bundles | Implemented | Verified offline model/runtime bundles, inventory preview and isolated import without changing active settings. |
| 75 | Shared model storage | Implemented | Immutable hash-addressed weights/projectors with verified version references and explicit CAS adoption; active, pinned and rollback references remain protected. |
| 76 | Canary update validation | Implemented | Candidates stage separately; real known print/table recognition must pass exact candidate/runtime/fixture checks before journaled activation. Failure/cancel retains active pointers; incomplete activation recovers before admission. |
| 77 | Per-project model pinning | Implemented | New local AI conversions record exact CAS models/projectors and runtime library identities; resolution/restoration verifies hashes without changing the global registry. Legacy manual pins describe future selection; HTTP aliases/OS drivers are not falsely pinned. |
| 78 | Update channels | Implemented | Stable/preview app channels, bounded notes and deferral, plus explicit project lock/version preferences; previews never silently alter project pins. |
| 79 | Safe storage cleanup | Implemented | Token-bound selected cleanup of verified known runtime ZIP caches; user data, active/pinned/rollback assets and unknown files protected. |
| 80 | Private-update credentials | Implemented | Fixed-publisher Windows Credential Manager entry, masked UI, zeroizing secret, scoped retrieval and no secret serialization. |
| 81 | Privacy dashboard | Implemented | Active in-app request inventory shows sanitized origins/purposes alongside updates, lookups and local locations; no payloads/tokens or OS-wide connection-monitor claim. |
| 82 | Encrypted workspaces | Implemented | Opt-in age-authenticated project snapshots with encrypted inventory, recovery keys, verified backup/restore/rotation and guarded managed work copies. Unlocked plaintext/native RAM and OS/driver/pagefile caches remain outside at-rest encryption. |
| 83 | Confidential session mode | Implemented | Retention-managed local-reader sessions with disabled lookup/context examples, verified external export and selected cleanup/recovery. Managed encrypted-workspace jobs keep app-owned scratch in guarded work copies; unknown/changed/busy files are retained and reported, with no secure-erasure promise. |
| 84 | Share-ready redaction | Implemented | Token-previewed isolated sharing copy rebuilds masked rasters, safe recognized content, metadata and evidence. Covered/ungrounded content is withheld, hidden Word correction baselines omitted and public settings use portable paths; private originals/history stay unchanged. |
| 85 | Inspectable diagnostic bundles | Implemented | Inspectable selected diagnostics; content excluded by default, safe allowlist summaries and token-bound export. |
| 86 | Plain-language troubleshooting | Implemented | Plain-language error checks and recovery actions in diagnostics plus existing NG remedies. |
| 87 | Verified repair tools | Implemented | Selected compiled-pinned assets and extracted members of verified publisher runtime archives can be repaired with damaged/concurrent bytes retained; registrations and active pointers stay unchanged. Custom/unverified archives are not authority; shared MSVC files remain installer-serviced. |
| 88 | Migration recovery | Implemented | Registered project/settings migration previews bind exact before/after bytes; durable backup/journal/rollback, stale-write checks and conflict-preserving recovery protect later edits. Unsupported transitions fail closed. |
| 89 | Failure-injection testing | Partial | Storage journal interruption/collision/disk-full simulations and real owned-child transport/crash/cancel tests plus interrupted-activation recovery; physical storage disconnect and real GPU-loss acceptance remain unperformed. |
| 90 | Stronger build provenance | Partial | Locked CycloneDX inventory, actual binary hashes and exact source/build records are available. Independent executable comparison failed; reproducibility is deferred for 0.5.0. The timestamp fix on main is not included in the tested release binaries. No successful reproducibility comparison or signed native attestation is claimed. |
| 91 | Command palette and shortcuts | Implemented | Searchable major actions, editable validated bindings and visible focus; review shortcuts require explicit review scope and leave text editing intact. |
| 92 | Screen-reader support | Partial | Native AccessKit integration, named settings/source controls, polite status updates and accessibility-tree tests; full desktop labels and Narrator reading/focus audit remain. |
| 93 | Readable-layout presets | Partial | Large/spacious/compact/high-contrast presets; full clipping audit at each viewport remains. |
| 94 | Precision source magnifier | Implemented | Keyboard-accessible bounded original-pixel crop, pan/zoom, selected source boundaries and disagreement markers; confirmed masks remain opaque and stale source fails closed. |
| 95 | Interface localization | Partial | English/Sinhala/Tamil catalogs cover core actions, settings descriptions and known-error remedies independently of OCR; specialist controls remain English and native-speaker/rendering acceptance remains pending. |
| 96 | Semantic Word styling | Implemented | Configurable semantic Word styles for headings/lists/captions/equations/references/code. |
| 97 | Additional text exports | Implemented | Markdown/HTML/LaTeX with escaping, source references and uncertainty. |
| 98 | Searchable PDF export | Implemented | Source-preserving raster PDF with block-aligned searchable Unicode and visible uncertainty; confirmed masks use sanitized raster/content instead of original PDF bytes, with image-only limits explicit. |
| 99 | Spreadsheet table export | Implemented | Real XLSX cells with literal values, source links, review sheet and merge warnings. |
| 100 | Word correction round-trip | Implemented | Tagged Word correction preview/import, conflict detection and one-revision selected apply. |
