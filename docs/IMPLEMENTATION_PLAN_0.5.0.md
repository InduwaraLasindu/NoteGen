# 0.5.0 release completion plan

The user has frozen feature scope: finish the work already in progress, validate it
and publish 0.5.0 promptly. The earlier requirement to complete every non-excluded
roadmap item before release is superseded. No new workstreams are added. Corpus
collection (#1) and Windows Authenticode signing (#10) remain excluded by request.

The [feature ledger](FEATURES_0.5.0.md) maps all 100 original proposals: 87 have
production implementations, 11 remain partial and 2 are excluded. These counts
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
| Exports and provenance | Word/text/PDF/XLSX with privacy guards and fail-closed correction import; locked SBOM and binary/source hashes; fresh independent Windows executable comparison and eligible native attestation workflows. Actual evidence remains CI-dependent. |

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

## Remaining release sequence

1. Freeze coherent source, finish only assigned regression fixes and capture the exact
   tested commit. Do not add measurement or other new feature workstreams.
2. Run format/strict Clippy and the all-feature/all-target suites twice on Windows and
   Linux, including the serial repeat. Run native UI/accessibility checks, printed OCR,
   font shaping/loading, privacy/workspace behavior, owned-reader reset/isolation,
   known canaries, real reader families and Maximum conversion for that source.
3. Build the Windows package with its recipe/provenance and compile both executables
   independently in a fresh job. Require exact executable hash comparison for release;
   record attestation eligibility honestly. Do not claim a match until CI produces it.
4. Merge only the validated source, then run the installer release workflow against its
   verified artifacts. Exercise clean install, released 0.4.1 upgrade, reinstall,
   cancellation, installed provenance, uninstall and retained user data.
5. Finalize README/release evidence and publish 0.5.0 only after those available gates
   pass. Verify published installer/portable assets, manifests and metadata hashes.

No earlier CI run, generated report template or implemented test is substituted for
observed final-source acceptance. The release may accurately describe frozen partial
scope without promising completion of all 100 proposals.
