# Verify build evidence

## 0.5.0 release evidence

The application binaries are the unchanged tested build from
[`19550373dd25afb36b21a37add0172b46da84117`](https://github.com/InduwaraLasindu/NoteGen/commit/19550373dd25afb36b21a37add0172b46da84117).
[Windows](https://github.com/InduwaraLasindu/NoteGen/actions/runs/41604450276/job/125810703182)
and [Linux](https://github.com/InduwaraLasindu/NoteGen/actions/runs/41604450276/job/125810703596)
functional/native producer jobs passed. The overall run failed its independent
executable comparison; there is no successful reproducibility report for this build.
The user has authorized deferring that GitHub testing blocker for 0.5.0. Feature #90
is partial, and no byte-for-byte reproducibility claim is made.

The portable ZIP SHA256 is
`0e1b7e281e9412fdd5fe24a20ddf71381b7485ac2bb6dca5ea86852e3824112f`.
Its `provenance` directory contains:

- `build-provenance.json`: exact application source, input pins and actual binary hashes.
- `notegen.cdx.json`: the locked CycloneDX dependency inventory.
- `provenance.sha256`: hashes of those metadata files.
- `reproducible-build.json`: compiler, recipe, source and executable hashes for one build.

These unsigned records identify inputs and bytes; they do not prove that another
build matched.

The release tag points to installer-packaging commit
`4bf4692d9320cc45026a48c757725fc34a815f8f`, a child of the tested application source.
Only three installer GUI entrypoints change to pass `--renderer glow`; application
Rust source, Cargo inputs and portable bytes remain unchanged. The installed app
therefore uses the existing OpenGL compatibility renderer. Portable users can
select it with `NoteGen.exe --renderer glow`.

Installer packaging adds app-local native runtime files and has its
own source, inventory and acceptance record. The final production installer passed
clean installation, binary inventory, CLI Word export, actual printed OCR,
eight-second OpenGL GUI startup, reinstall, actual 0.4.1 upgrade and uninstall/data
retention checks. Its SHA256 is
`7049f92340f2c546f740c464081937d5b17e7ff3abcde66819440a4d2a45e0ec`.
The installer AI downloading branch was not rerun. Publication and uploaded asset
download verification remain the final steps. Release notes and manifests
must distinguish the application source from the installer packaging source and
later documentation changes.

The timestamp-metadata fix in main commit
`74e3d5b899e65cf2d1092c41619bffbcd248ec25` enables `rust-embed` deterministic
translation timestamps. It is not included in the tested 0.5.0 application binaries.
The existing portable provenance must not be relabelled as built from that commit.

## Retained independent-build workflow

The retained workflow uses Rust 1.99.1, locked dependencies, release optimizations,
disabled incremental compilation, remapped paths and MSVC `/Brepro`. A fresh
Windows job compares actual executable SHA256 values against the first portable
artifact and checks source/compiler/recipe/runner identity. A mismatch fails the
workflow. Future successful evidence must come from an actual comparison; the
0.5.0 release exception does not change the comparison result or normalize bytes.
ZIP timestamps, installers, third-party binaries and other toolchain/SDK/runner
versions are outside its comparison scope. Synthetic comparator tests do not
substitute for independently compiling matching executable bytes.

## Native build attestations

No signed native attestation is claimed for this 0.5.0 payload. Checksums and
unsigned JSON records are not cryptographic attestations. Windows Authenticode
signing remains excluded by request.

On eligible repositories, the retained trusted main-branch workflow can use a
commit-pinned `actions/attest` action with OIDC permissions to attest the portable
ZIP and successful comparison report. Pull-request builds do not publish these
attestations. [GitHub's availability rules](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations)
require Enterprise Cloud for private/internal repositories. An eligible repository
may deliberately configure the service in a later verified build; no paid service
or spending change is enabled for this release.
