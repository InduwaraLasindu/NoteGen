# Rust verification record

Implemented behavior, mocked/protocol tests and physical quality evidence are separate.
Use [Actions](https://github.com/InduwaraLasindu/NoteGen/actions/workflows/verify-and-build.yml)
for exact Windows/Linux results, native visual evidence and the executable download.

## 0.5.0 application evidence and deferred gates

The release application source is
[`19550373dd25afb36b21a37add0172b46da84117`](https://github.com/InduwaraLasindu/NoteGen/commit/19550373dd25afb36b21a37add0172b46da84117).
The successful producer jobs in
[run 41604450276](https://github.com/InduwaraLasindu/NoteGen/actions/runs/41604450276)
are:

- [Windows verification](https://github.com/InduwaraLasindu/NoteGen/actions/runs/41604450276/job/125810703182):
  strict formatting/Clippy, complete ordinary suites twice (including a serial
  pass), native font and isolated credential checks, packaged scanned-PDF OCR,
  portable desktop startup, actual CPU reader installation/inference, warm-reader
  reset/reuse/isolation and Maximum Accuracy conversion.
- [Linux verification](https://github.com/InduwaraLasindu/NoteGen/actions/runs/41604450276/job/125810703596):
  strict formatting/Clippy, complete ordinary suites twice, actual raster-PDF OCR
  and native desktop/source-review captures.

The overall run failed its separate independent executable comparison. No matching
independent-build report or signed native attestation is claimed. The user has
explicitly authorized deferring this GitHub testing blocker for 0.5.0. Later main
commit `74e3d5b899e65cf2d1092c41619bffbcd248ec25` contains a timestamp-metadata fix;
that fix is not included in these release binaries and has not passed the full
new-source verification path. Feature #90 is therefore partial.

The unchanged portable package SHA256 is
`0e1b7e281e9412fdd5fe24a20ddf71381b7485ac2bb6dca5ea86852e3824112f`.
Its 106 binary subjects were checked against packaged provenance. Additional local
checks of this exact MSVC package passed CLI version, Word self-test and two
independent printed OCR/table conversions; each retained 1250 in an editable Word
table. These checks do not replace the separate CI evidence or measure handwriting
quality, GPU performance or full accessibility.

The release tag uses installer-packaging commit
`4bf4692d9320cc45026a48c757725fc34a815f8f`, with unchanged Rust/Cargo application inputs
and three installer GUI entrypoints selecting `--renderer glow`. The actual
installer passed its 106-subject payload inventory, CLI Word export and real printed
OCR checks. On Intel HD 4000, the default WGPU desktop panicked during surface
configuration; launching the same binary with `--renderer glow` stayed alive for
12 seconds. This is a bounded startup observation, not a general stability result.
The final production installer, built with that existing compatibility option,
passed clean installation and verification of all 106 original binary subjects,
CLI Word export and actual printed raster-PDF OCR. The installed OpenGL GUI stayed
alive for eight seconds after clean install, same-version reinstall and actual
0.4.1-to-0.5.0 upgrade. Reinstall preserved the selected runtime and synthetic user
documents/models. Both uninstall paths removed the executable, registration and
shortcuts while retaining synthetic user data. The actual prior 0.4.1 CLI/version
and upgrade were tested; its GUI was deliberately not launched because it ignores
data-directory isolation.

Installer SHA256:
`7049f92340f2c546f740c464081937d5b17e7ff3abcde66819440a4d2a45e0ec`.
The installer AI download/integration branch was not rerun during this offline
acceptance. The earlier source-195 native AI checks are separate evidence.
The 0.5.0 installer is packaged separately from the immutable application binaries.
[Version 0.5.0](https://github.com/InduwaraLasindu/NoteGen/releases/tag/v0.5.0) is published. All 13 release assets were downloaded again and their SHA256 hashes and sizes matched the finalized local artifacts.
The [Narrator audit](ACCESSIBILITY.md), broad natural-handwriting measurements and
additional physical hardware/language acceptance remain unclaimed. The historical
results below apply only to their named source snapshots.

## Historical verified build — 31 October 2026

[Windows/Linux run](https://github.com/InduwaraLasindu/NoteGen/actions/runs/41168595803) at source commit
[`1dd2606`](https://github.com/InduwaraLasindu/NoteGen/commit/1dd2606040305fc12c06b8396004c81485c0917b)
passed: **65 ordinary tests on each OS**, strict fmt/clippy, native Linux desktop captures,
Windows runtime/model installation, three explicit real-model adapter tests, Maximum
Accuracy conversion, packaged raster-PDF OCR/table export and startup of the desktop
extracted from the portable ZIP. The final Windows Maximum run took 541.19 seconds on
that CPU runner. No RTX measurements are implied.

[Download the verified Windows artifact](https://github.com/InduwaraLasindu/NoteGen/actions/runs/41168595803/artifacts/13031096506) (85,938,095 bytes;
Actions retention through 30 November 2026). It contains the portable ZIP and its SHA256
file. Artifact archive digest: `37956f4dd333aa61e71b242ae63ff73dafc17309c9562ca583e34477eb2c29d9`.
This digest identifies the outer Actions artifact, not the inner portable ZIP.

## Iteration gates

| Iteration | Evidence |
|---|---|
| 1 | 13 core tests + format/clippy: simulated 4/8/16/32 GiB profiles, busy VRAM, parameter validation, independent consensus and numeric disagreement |
| 2 | 31 tests: real PDFium/Tesseract raster-only PDF conversion, editable table, resume, DOCX merges/media, local SSE, faults/OOM/redirects/cancellation |
| 3 | 39 tests with all features/targets + format/clippy: Advanced tabs, text/table/figure review, light/dark/reduced motion, queue/worker lifecycle, bounded logs, human corrections |
| 4 | Additional real-model mixed-format regression, CPU-only isolation, manual selection, Balanced scheduling and draft-save protection; Windows/Linux/package gates in CI |
| 5 | 61 ordinary tests + format/clippy locally: motion/reduced-motion render comparisons, animation timing and frame budgets, backward-compatible appearance settings, invalid AI grounding recovery and raw response evidence |
| 6 | 65 ordinary tests + format/clippy locally: Fast conversion with damaged/invalid unused AI registrations, selected-reader checksum rejection, handwriting/preset validation, CLI settings preservation and honest single-family agreement labels |

The 31 October source passed all 65 ordinary tests with all features/targets on Linux,
plus strict format and clippy checks. The native desktop was started under an authenticated
virtual X display; workspace, Advanced and review were captured, and the actual scanning
graphic was recorded for four seconds. The screenshot harness decodes its output before
reporting success. A compiled-CLI raster-PDF conversion produced an editable Word file.
Cross-platform and packaged-executable status is recorded by the Actions run, independently
of these local checks.

The native fixture contains **only raster image data**, known printed text and a
two-column table with cell 1250. No selectable PDF text shortcuts OCR. Rust draws the
fixture. NOTEGEN_FIXTURE_DIR exports its PNG/PDF for independent package tests.

## Real inference performed locally

- Official GLM-OCR Q8 + matching Q8 projector, llama.cpp b11998, CPU: the known printed
  sentence was recognized. Table Recognition on the source crop produced editable cells
  including 1250. Text-only whole-page mode omitted the table; NoteGen preserves table
  regions from other readers and uses the specialist table protocol.
- LightOnOCR-3 1B Q8 + matching F16 projector from richardyoung's pinned conversion,
  CPU: the full page returned the known text and HTML table. A crop returned Markdown
  inside HTML tags; the real test caught this and the parser now reconstructs its cells.
  The repeated live test passed. Grounding markers were absent in this sample, so the
  adapter emits an explicit source-grounding review warning.
- Qwen3.5 2B Q4_K_M + matching F16 projector, b11998 CPU: full-page and crop inference
  passed in the explicit live test. It read 1250, rebuilt the table and described the
  bar chart as a visual representation of sales. Its JSON supplied zero-sized boxes.
  The regression now retains text/table data without invented coordinates and attaches
  an ungrounded figure to the full source page with an uncertainty/review warning.
  Valid coordinates remain unchanged; malformed table structure is still rejected.
  Raw reader responses are retained separately under the local project/runtime
  `responses` directory for diagnosis, without putting document text in live log messages.
- A release-CLI Maximum Accuracy conversion completed with Tesseract, LightOnOCR-3,
  Qwen3.5 and GLM: four full-page families plus three crop readings. Native Word XML
  contains the reconstructed table (1250) and two retained source images. This run
  took 197.69 seconds on CPU with eight inference threads and a resumed printed-OCR
  checkpoint; it is not a cold-start benchmark. Agreement was 84.0%, and source/grounding
  warnings correctly required review. The native review capture shows this real result.

These demonstrate native inference/adapters on synthetic print, tables and one simple
chart. They do not measure broad handwriting accuracy or diagram understanding. Model revisions/SHA256 are in
rust/downloads.json. Large model tests are opt-in and were invoked explicitly.

### Public handwriting sanity check

A real English handwritten line from [Teklia/IAM-line](https://huggingface.co/datasets/Teklia/IAM-line),
test split row 0, was rasterized into a PDF and converted with the release CLI in Maximum
Accuracy on CPU. LightOnOCR-3 1B, Qwen3.5 2B and GLM-OCR each matched all reference letters
and digits after NFKC normalization, case folding and removal of punctuation/spacing.
The printed Tesseract baseline had 36 edits over the 69 normalized reference characters.
This is a limited lexical comparison, **not standard CER** and not a broad accuracy score.
The seven readings, disagreement warnings and review requirement were retained; Word
export completed in 91.64 seconds. Possible training overlap is unknown. The source image
is not bundled. Its digest and reproducibility metadata are in [the QA record](QA_2026-10-31.json).

## CI/package verification

Both operating systems install PDFium/Tesseract, run Rust format, clippy with all
features/targets, and ordinary tests. Linux also converts the scan through the compiled
CLI, starts the real desktop and captures workspace/Advanced/review screenshots.
Windows builds release executables, verifies native runtime installation, packages all
required printed-OCR dependencies/licenses, and checks a raster PDF through the packaged
CLI for 1250 plus a real Word table. It starts the desktop extracted from the portable ZIP.

The first motion run passed all Windows AI checks, including all three real adapters
and the Maximum Accuracy pipeline (585.91 seconds on that CPU runner), then exposed a
packaging-only PowerShell JSON case-collision in transitive dependency metadata. The
packager now uses a case-sensitive hashtable and explicitly requires PowerShell 7.
Its production license-inventory block was exercised against all 581 actual Cargo
packages in PowerShell 7.6.9; both `USB` and `usb` remain distinct and license files
were collected successfully. The final Windows run passed the complete ZIP and extracted-desktop startup gates.

Screenshots show the actual Rust app in the test machine's CPU mode. They do not depict
an RTX machine or fabricated performance telemetry.

## Reproduce

Install Tesseract with eng/osd and PDFium chromium/8125; set NOTEGEN_PDFIUM to its library.

~~~shell
cargo fmt --all --check
cargo clippy --locked --all-features --all-targets -j 2 -- -D warnings
cargo test --locked --all-features --all-targets -j 2
cargo run --features desktop --example ui_capture -- build/workspace.png workspace
~~~

Use xvfb-run -a for native GUI capture on headless Linux. For actual AI testing, set
NOTEGEN_LIVE_ROOT, NOTEGEN_LIVE_ID (glm-ocr, lighton3-1b or qwen35-2b), NOTEGEN_LIVE_WEIGHTS,
NOTEGEN_LIVE_PROJECTOR and NOTEGEN_LIVE_SERVER, then:
cargo test --test engine live_model -- --ignored --nocapture.

## Remaining target-PC acceptance

No physical RTX GPU or access to your Windows PC was available. GPU-profile tests are
simulations. Actual layer-offload verification is implemented, but peak RAM/VRAM,
latency, driver compatibility and memory-pressure recovery need target-device runs.
The 4B/9B/27B catalog entries have not all been physically inferred in this session.

Evaluate a labeled private corpus of clear print, cursive, terrible handwriting, blur,
mixed scripts, merged/borderless tables, diagrams and cross-page context. Record CER/WER,
exact cell/topology accuracy, grounded diagram claims, false agreement, peak memory,
seconds/page and all model/runtime/driver hashes. Agreement is not calibrated accuracy;
correlated readers can share mistakes. Unreadable source information remains uncertain.

The Windows gate also invokes the real model installer and native CPU inference for
GLM-OCR, LightOnOCR-3 1B and Qwen3.5 2B, then a Maximum Accuracy conversion. Its generated
fixture now includes a bar chart; the vision adapter must preserve a valid source box,
visible description and contextual interpretation. See the run for whether these gates
have passed; inclusion in a workflow alone is not evidence of a successful test.


## 0.3 restored-source progress/setup gate

The original Studio candidate is the baseline. Focused tests cover learned stage timing,
page-weighted queues, changing work budgets, non-completion on cancellation/failure,
paused ETA clocks, isolation from arbitrary log counters, setup state application,
immutable matching model metadata, hardware-tier recommendations, verified newer-reader
reuse, downloader resume/restart/checksum/cancellation, and bounded runtime extraction.
The real PDFium/Tesseract test also verifies selected-page preflight and structured progress
through successful Word export. Windows `Smoke-AI.ps1` invokes native runtime installation
and downloads/activates three real readers through `notegen-cli setup` before inference.

The new candidate's final result and release provenance are recorded after CI and installer
acceptance. Earlier test counts above describe earlier revisions.

Local restored-source candidate: **94 passed, 1 explicitly ignored real-model test**.
`cargo fmt --all --check` and strict all-feature/all-target Clippy passed. The ordinary
suite includes native raster-PDF/Tesseract/Word conversion and the new progress/setup
regressions. Real Windows runtime/model installation, inference, packaging and installer
acceptance are separate CI gates; their evidence is attached to the successful release.

The final follow-up uses the operating system certificate store for HTTPS downloads
(TLS verification remains mandatory). A real GLM-OCR model download exercises that
path. Setup reserves model work before runtime loading so the overall bar does not
approach completion before large model downloads begin; a regression test covers it.


## Released 0.3.0 — final verification (2026-11-01)

[Download the tested installer](https://github.com/InduwaraLasindu/NoteGen/releases/tag/v0.3.0).

| Gate | Result | Evidence |
|---|---|---|
| Formatting, strict Clippy and ordinary Rust tests | Passed on Windows and Linux; 94 tests per OS, plus the separately invoked model gate | [Source workflow 41306134366](https://github.com/InduwaraLasindu/NoteGen/actions/runs/41306134366) |
| Native desktop, source/review screens and font support | Passed; Windows loaded/rendered the exported TrueType font | Source workflow artifacts |
| Native one-click setup | Downloaded, SHA256-verified and registered the CPU runtime and GLM-OCR, LightOnOCR-3-1B and Qwen3.5-2B | Real Windows model evidence |
| Live AI and Maximum Accuracy | All three reader tests passed, including writer-reference requests, table crops, diagram preservation and model-major Word conversion | Source workflow; Maximum Accuracy synthetic page took 889.22 seconds on the CPU runner |
| Installed application | Launch, real PDF OCR, editable Word table, writer/glyph teaching, font/profile export and Windows font rendering passed | [Installer workflow 41312695109](https://github.com/InduwaraLasindu/NoteGen/actions/runs/41312695109) |
| Upgrade and uninstall | Actual 0.2 → 0.3 upgrade, same-version reinstall and uninstall passed; models, documents and handwriting profiles preserved; selected runtime preserved across reinstall | Installer validation evidence |

The installer packages the immutable application build from
`0a873138d7eac358ba3ff7ae9e9e6f2de793f53e`. Installer controls and acceptance use
`e9b73fa16cd436b851289b74b27775f1b851c46c`; only installer/test metadata differs.
The installer harness now uses bounded process waits and stage logs. This avoids an
unbounded wait for unrelated descendant processes; its complete retry passed in under
a minute. Binary provenance, ZIP/artifact digests and all installer validation records
are included in the release's `installer-manifest.json`.

Installer SHA256:
`9c64831e0b10e8d32bb57b3faf89df7a242dbb089533cb6cd3f00545de02501e`.

These are synthetic worksheet and feature checks, not a broad handwriting benchmark.
Actual RTX throughput/VRAM and difficult natural-handwriting accuracy remain unmeasured.
The installer is unsigned. No optimized Linux release package is claimed; Linux evidence
covers the source suite and native desktop/Word checks.

## 0.4.0 release gates

`rust/tests/experience.rs` covers publisher release selection (including stable marker releases with no binaries), asset trust/digests, app-version checks, preview filtering, checkpoint compatibility, complete settings help, and rejection of incorrect tuning trials. CI also runs the measured hardware probe against a real installed reader, captures all new settings screens, and performs a real default-Maximum AI setup through the compiled installer. Installer QA upgrades the released 0.3.0 installer, exercises explicit cancellation, and checks retained AI/model/writer data. Live smoke input is capped at 768 pixels for bounded CI; production input defaults are unchanged. Use `scripts/Smoke-AI.ps1 -FullQuality` for native-resolution manual validation.
