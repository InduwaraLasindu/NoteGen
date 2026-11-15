# 0.5.0 release completion plan

The user has frozen feature scope: finish the work already in progress, validate it
and publish 0.5.0 promptly. The earlier requirement to complete every non-excluded
roadmap item before release is superseded. No new workstreams are added. Corpus
collection (#1) and Windows Authenticode signing (#10) remain excluded by request.

The [feature ledger](FEATURES_0.5.0.md) maps all 100 original proposals: 86 have
production implementations, 12 remain partial and 2 are excluded. These counts
describe code and recorded limits, not universal quality or a completed release gate.
The [verification record](TESTING.md) owns exact source/run/test evidence; older
successful snapshots never validate subsequent changes.

## Frozen implementation scope

| Area | Current production paths and limits |
|---|---|
| Source privacy | Preparation without recognition, confirmed source-bound covers, masking before every OCR/AI crop, context omission, conservative output withholding, sanitized sharing/export projections and stale-source rejection. Originals and private history remain private. |
| Review and geometry | Durable page/context/writer history, configurable focus-scoped shortcuts, responsive review actions, original-pixel magnifier, script suggestions, confirmed language regions, line repair/reread, table-grid mappings and coordinate-backed heatmaps. Missing geometry is not invented. |
| Recognition detail | Literal code OCR/evidence and whitespace-sensitive consensus; bounded chemical element/charge/reaction checks; explicit footnote/citation/bibliography relationships and Word footnotes. No automatic semantic correction. |
| Projects and queue | Stable source catalog, reversible page layout transactions, compatible duplicate reuse/comparison/private forks, supported project/settings migrations with backups and rollback, urgent queue yielding at durable checkpoints and original-job resume. |
| Fonts and languages | Confirmed scalar/cluster/ligature/alternate shapes, cmap/GSUB generation and proof sheets; no inferred contextual joining. Core English/Sinhala/Tamil actions, settings help and known-error catalogs are separate from OCR resources. Specialist controls still include English. |
| Private workspaces | Authenticated age snapshots, recovery/backup/rotation, managed work copies and guarded app-owned temporary data; explicit retention cleanup/recovery. Unlocked native RAM and OS/driver/pagefile caches remain outside at-rest guarantees. |
| AI assets and setup | Managed verified downloads with scheduling/rate/network policy, immutable shared objects, version references, exact project locks/restoration, known print/table canaries, journaled activation recovery and selective verified runtime repair. |
| Performance and transparency | Optional max-one warm reader with reset/TTL/release and cold private/trial jobs; bounded preparation overlap; resource reservations and honest sampled memory/timing observations; source/license capability cards and active in-app request inventory. |
| Exports and provenance | Word/text/PDF/XLSX with privacy guards and fail-closed correction import; locked SBOM and binary/source hashes. Independent comparison failed and is deferred for this release; no reproducibility or native attestation claim. |

Unused calibration/memory settings and the unfinished writer-source sidecar prototype
have been removed. Existing scorecards, calibration-bin reports, writer-separated
dataset checks and sampled hardware observations remain available. No ignored setting
or draft module is presented as a finished measurement workflow.

## Deferred scope and external acceptance

The following do not block the frozen release by reopening implementation scope:

- Deployable hash-bound calibration/abstention policies, measured automatic regional
  reader routing, paired profile/baseline evaluation and learned memory admission.
- Automatic language-region classification and measured tested-language/model quality
  claims beyond available evidence; source/license cards explicitly identify unknowns.
- Physical multi-GPU comparisons, instantaneous RAM/VRAM peak guarantees, real device
  removal/GPU-loss acceptance and broad natural-handwriting measurements.
- Complete desktop localization, native-speaker/font visual review, the full Narrator
  audit and every panel/viewport/readability-preset combination.

Unsupported formats/grammars, archive variants and geometry precision remain explicit
at their production boundaries. No secure-erasure, universal accuracy or 100% stability
guarantee is inferred from automated tests. The original roadmap stays unchanged as a
record of proposals; deferred work is for future releases.

## Release sequence after the GitHub testing deferral

The user authorized skipping the blocked GitHub testing path and publishing the
completed update. This supersedes the earlier requirement to wait for independent
reproducibility before publication. It does not turn a failed comparison into a pass.
Feature #90 remains partial; no new application development is added.

1. Release the immutable application binaries from source
   `19550373dd25afb36b21a37add0172b46da84117`, whose Windows/Linux functional and
   native checks passed. Keep their existing source records and checksums intact.
2. Record that overall run `41604450276` failed independent executable comparison.
   The later timestamp-metadata fix on main is outside these release binaries.
   Do not label the application as built from the newer main revision.
3. Use installer-packaging source `4bf4692d9320cc45026a48c757725fc34a815f8f` for the release
   tag. Its only changes from the tested application source select the existing
   `--renderer glow` compatibility renderer in three installer GUI entrypoints.
   Rust/Cargo inputs and portable binaries stay unchanged. Portable users with
   older GPUs can select the same renderer with `NoteGen.exe --renderer glow`.
   Package the tested application into the Windows installer and record the exact
   installer recipe/source, payload hashes and actual acceptance results separately.
   The final installer passed clean installation, all 106 original binary checks,
   Word export, actual printed OCR, eight-second OpenGL GUI startup, reinstall,
   actual 0.4.1 upgrade and uninstall/data retention. Installer SHA256:
   `7049f92340f2c546f740c464081937d5b17e7ff3abcde66819440a4d2a45e0ec`.
   The installer AI downloading branch was not rerun; source-195 native AI
   evidence remains separate.
4. Publish the release at the installer-packaging source while identifying the
   separate tested application source in the manifest. Attach the available
   checksums/build evidence, and verify the uploaded assets can be downloaded and
   match their recorded hashes. No paid cloud services or spending changes are used.

Earlier successful snapshots never validate changed application code. Release
notes and manifests must distinguish the tested application, installer packaging
and any later documentation changes. This release does not claim all 100 proposals,
byte-for-byte reproducibility, signed native attestations or unperformed tests.
