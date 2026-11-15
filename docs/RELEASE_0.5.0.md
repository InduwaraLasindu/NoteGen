# NoteGen 0.5.0

This release expands local source review, private workspaces and verified AI setup.
Its scope is frozen at the user's request. The [100-item ledger](FEATURES_0.5.0.md)
records 86 implemented proposals, 12 partial items and 2 exclusions; it is not an
all-100-complete claim. The [verification record](TESTING.md) and published manifests
identify the exact tested source and observed acceptance results.

Review gains configurable keyboard save/approve/next/defer controls, durable revision
history and a bounded original-pixel magnifier. Source tools add image-based script
suggestions, confirmed paragraph/line OCR languages, repaired line boundaries and
table-grid previews. Fine disagreement overlays use actual supplied word/cell boxes;
missing precision remains explicit. Literal code mode preserves whitespace and raw
evidence without executing or repairing code. Numeric and bounded chemistry checks
flag suspicious values, element counts and reaction charge without rewriting them.

Projects now support reversible reorder, insert, archive, split and merge with stable
source identities. Exact-compatible duplicate reuse, comparison and private revision
copies retain evidence, history and locks. Supported project/settings migrations use
previewed before-images, durable journals and conflict-preserving recovery. Urgent
queue inputs can run after safe saved checkpoints; interrupted work resumes with its
original settings and output. Active native requests finish before yielding.

Privacy tools can prepare scans before any recognition, confirm source-bound covers
and mask every OCR/AI view. Covered, intersecting or ungrounded text is withheld;
stale sources stop processing. Sharing creates a separate sanitized source/text/project
copy. Word omits hidden correction baselines for masked exports, and PDF uses masked
rasters instead of original PDF bytes. Private originals and undo history stay private.
The dashboard reports active in-app request origins without payloads or credentials.

Optional encrypted workspaces use authenticated age snapshots with recovery keys,
verified backups, rotation and guarded managed work copies. Retention sessions provide
verified external exports and selected cleanup/recovery reports. Managed encrypted
jobs keep app-owned scratch in the guarded work copy; unlocked plaintext/native RAM,
OS/driver caches, pagefile and crash dumps remain outside encrypted-at-rest protection.
Cleanup retains unknown, changed or busy files and is ordinary deletion, not secure erasure.

AI setup adds a persisted download manager with bandwidth limits, network-cost policy,
UTC windows and verified pause/resume. Shared hash-addressed models and exact project
locks preserve version identity. Model/runtime candidates must pass real known
print/table canaries before journaled activation; repair can restore selected verified
runtime members while retaining damaged bytes and registrations. Offline bundles and
source/license capability cards remain available; unknown language measurements and
estimated memory figures are labelled. Stable/preview channels and project preferences
never silently change a project pin.

Optional warm-reader reuse holds at most one compatible owned reader for 1–60 seconds,
resets its context and offers immediate release. Private sessions, masked jobs and
hardware trials use fresh processes. Rendering/OCR overlap remains opt-in and bounded
by CPU/RAM admission with sequential fallback. Native working buffers may remain in
RAM until the reader is released; admission estimates are not OS quotas.

Exports include semantic Word styles, bounded editable equations, confirmed footnotes
and references, Markdown/HTML/LaTeX, literal Excel cells and searchable PDF. Correction
import previews conflicts and rejects unsupported Word content rather than silently
dropping it. Confirmed Unicode scalar, cluster, ligature and alternate handwriting
shapes can generate cmap/GSUB font packs and proof sheets; missing joining/positioning
is not invented. English/Sinhala/Tamil catalogs cover core actions, settings help and
known-error remedies independently of OCR; specialist controls remain English.

The application binaries come from tested source
[`19550373`](https://github.com/InduwaraLasindu/NoteGen/commit/19550373dd25afb36b21a37add0172b46da84117).
The release tag uses installer-packaging commit
[`4bf4692d`](https://github.com/InduwaraLasindu/NoteGen/commit/4bf4692d9320cc45026a48c757725fc34a815f8f),
whose Rust source and Cargo inputs are unchanged from that application build. Its
only packaging change launches the existing OpenGL compatibility renderer from
the installer and shortcuts. Portable users with older GPUs can run
`NoteGen.exe --renderer glow`.

The complete Windows/Linux functional suites passed twice, alongside native OCR,
reader, warm-reader isolation and Maximum Accuracy checks. The overall
[verification run](https://github.com/InduwaraLasindu/NoteGen/actions/runs/41604450276)
failed its independent executable comparison; it is not a fully green CI run.

At the user's request, the blocked GitHub testing path and reproducibility acceptance
are deferred for this release. The later timestamp-metadata fix on main is not part
of these binaries. Checksums, the locked dependency inventory and source/build records
identify the actual payload; they do not establish reproducibility or a signed
attestation. The final production installer passed clean installation, all 106
original binary checks, Word export, actual printed raster-PDF OCR, same-version
reinstall, actual 0.4.1-to-0.5.0 upgrade and uninstall/data retention checks. The
installed GUI stayed alive for eight seconds in each tested stage using the shipped
OpenGL renderer. Installer AI downloading was not rerun; the earlier native AI
producer-job evidence remains separate. [Version 0.5.0](https://github.com/InduwaraLasindu/NoteGen/releases/tag/v0.5.0) is published, with all 13 downloaded assets verified against their SHA256 hashes and sizes.

Installer SHA256:
`7049f92340f2c546f740c464081937d5b17e7ff3abcde66819440a4d2a45e0ec`.

Deployment calibration, measured automatic reader routing, paired personalization
evaluation and learned memory admission are deferred. Broad handwriting accuracy,
physical multi-GPU/fault acceptance, native-language/font visual review and the full
Narrator/viewport audit remain unclaimed. Corpus collection and Windows Authenticode
signing were excluded by request; the installer is unsigned.

See the [workbench guide](WORKBENCH.md), [source tools](SOURCE_REVIEW.md),
[project operations](PROJECT_OPERATIONS.md), [accessibility](ACCESSIBILITY.md) and
[installation guide](INSTALLER.md) for workflows and limits.
