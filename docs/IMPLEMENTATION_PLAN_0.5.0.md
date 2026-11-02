# 0.5.0 completion plan

The active goal is all 100 proposals, followed by repeated validation and publication.
The implementation ledger records current evidence; it is not a claim that every
proposal is complete. No release is published while required code gaps remain.
Corpus collection (#1) and Windows signing (#10) remain excluded by the user's
earlier instruction. Other skips require a documented overlap or a reason the
original proposal does not require the extra scope.

## Current validation baseline

The current baseline passed two ordinary-suite runs on both Windows and Linux,
native Review visibility/click/scroll checks, Windows font and credential checks,
optimized compilation, packaged OCR and desktop startup. A real Qwen table request
then exposed invalid merge claims. The exact raw response is now a regression
fixture: the reader adapter retains all cells, removes that affected table's merge
claims, and requires review. Strict project/import validators remain strict.
Consensus must preserve this uncertainty even when other readers agree on text.

The new recovery is being validated independently while the remaining feature
work is integrated. Baseline CI success is never substituted for testing a later
source snapshot.

## Implementation sequence

| Workstream | Required outcome |
|---|---|
| Redactions (#27, #84) | Confirm source-bound covers; mask all recognition views; withhold intersecting or ungrounded output; preserve redaction placeholders across review/export; produce an isolated sharing copy with sanitized source, text, metadata and evidence. |
| Fonts (#59) | Explicit confirmed Unicode scalars, clusters, ligatures and alternatives; valid cmap/GSUB tables; no inferred unsampled joining; real shaping tests and proof sheets. |
| Warm readers (#66) | One owned process, complete compatibility key, fresh context barrier, short TTL, memory admission, immediate release and correct cancellation/log ownership; real same-PID isolation tests. |
| Downloads (#73) | One manager for verified asset transfers, persisted jobs, bandwidth budget, metered/unknown controls, UTC windows, pause/resume/cancel, partial-range recovery and no credential persistence. |
| Canary activation (#76) | Stage and verify candidates; run a known recognition fixture before activation; bind results to candidate/fixture hashes; retain working pointers on failure or cancellation. |
| Encryption (#82) | Maintained authenticated encryption, bounded inputs, encrypted private manifest, passphrase and recovery workflow, verified backups/rotation, atomic commits and explicit managed plaintext lifecycle. |
| Asset identity (#75, #77, #78) | Immutable content-addressed objects, version-aware references, exact project locks/restoration and project version/channel policies. |
| Project transactions (#30, #46, #87, #88) | Stable source-page identities, reversible layout changes, explicit compatible duplicate reuse/compare, extracted-runtime repair and supported migration backup/journal/rollback. |
| Measured recognition (#3, #5, #53, #63, #72) | Deployable calibration/routing artifacts, paired personalized evaluation, conservative learned memory admission and source-bound capability cards; distinguish measurements from estimates. |
| Recognition detail (#6, #11, #12, #16, #17, #20, #24, #32, #37) | Better preflight/script/language routing; bounded chemistry/code handling; confirmed references; editable line/table geometry; retain geometry actually supplied by readers. |
| Desktop workflows (#9, #45, #81, #91, #94) | Learned-example history, durable-stage urgent scheduling, active in-app connection/operation inventory, configurable shortcuts and a bounded full-resolution magnifier. |
| Accessibility/locales (#92, #93, #95) | Complete named/focused/status controls, viewport/preset matrix, externalized interface/help/error catalogs and explicit native-language/Narrator acceptance records. |
| Reliability/provenance (#89, #90) | Deterministic storage/process/GPU-loss and interrupted-activation tests, isolated build comparison and verifiable build attestations where the connected service supports them. |

Shared-file ownership is coordinated before edits. New helpers do not count as
complete features until production entry points, CLI/desktop controls, persistence,
undo/recovery and appropriate tests are wired.

## Overlap decisions to verify against original wording

The following are evidence-backed candidates for completion without adding an
extra feature that the proposal did not require:

- #19 already offers keep/remove for recurring elements; mapping to Word sections
  was an alternative, not an additional mandatory choice.
- #21 can improve uneven lighting without promising recovery of destroyed detail.
- #23 has editable four-corner correction through the CLI; a draggable editor was
  not specified by the proposal.
- #39 already flags cross-page inconsistencies without forcing normalization.
- #65 reserves application capacity; a hard operating-system quota is extra scope.
- #70 overlaps selected page/context/region rereads; automatic dependency discovery
  must not be silently added to the original requirement.
- #71 reports selected/reused/download bytes and a conservative safe disk bound;
  an unknown archive's exact expansion is not asserted.
- #83 supplies explicit retention and cleanup/recovery reports. Encryption and
  forensic erasure of OS/native caches are separate concerns.
- #49's HEIC support was conditional on a reliable decoder. Supported TIFF
  color/depth limits must remain explicit.

These decisions do not excuse the separate missing code in the workstream table.
Ledger statuses change only after the relevant path and tests are verified.

## Validation and acceptance

Each completed implementation receives focused regression/contract tests and then
the full all-feature/all-target suite twice on Windows and Linux, including a
serial repeat. Native gates cover real printed OCR, font loading/shaping, reader
startup/isolation, three reader families, Maximum conversion, packaging and the
actual installer. Install acceptance includes a clean install, the released 0.4.1
upgrade, reinstall, cancellation, provenance, uninstall and retained user data.

External acceptance remains distinct from missing workflow code. Permission-cleared
held-out labels are required for real calibration, routing and personalization
claims; collecting the excluded corpus is not performed. Physical multi-GPU,
target-device peak/offload behavior, native-language shaping/translation and
Narrator reading require their corresponding environment and observed evidence.
Synthetic CPU tests are not substitutes for those measurements. No universal
accuracy, secure-erasure or 100% stability guarantee is inferred from passing tests.

After all required code and available acceptance gates pass, update the minimal
README and final verification record, merge the exact tested source, run the
installer/release workflow, and verify the published 0.5.0 assets and manifest.
