# Verify build evidence

The Windows verification workflow compiles the two application executables using
Rust 1.99.1, the locked dependencies, release optimizations, disabled incremental
compilation, remapped build/source/dependency paths and the MSVC `/Brepro` linker
option. The recipe refuses an unclean source checkout or ambient compiler flags.
Its record includes the exact source commit, recipe hash, Cargo.lock hash, compiler,
runner image and actual executable hashes. A recipe record alone is not evidence
that another build matched.

The pinned `rust-embed` dependency enables `deterministic-timestamps` for the
translations embedded by `age`. Their created/modified metadata is zero, so
cached dependency outputs cannot retain a previous source commit's timestamp.
The build record still retains the current source commit and timestamp.

After both operating systems pass the required tests and native acceptance,
another Windows runner checks out the same source and compiles into an empty
target directory. It receives the first portable artifact, verifies each declared
hash against the actual archived executable, checks the recipe/environment
identity, then compares the independently compiled executable bytes. A mismatch
fails verification and prevents the installer release. No reproducibility claim
is made for ZIP timestamps, the installer, third-party native dependencies or
other compiler/SDK/runner versions.

The release includes:

- `windows-reproducibility.json`: exact matching subjects and comparison scope.
- `portable-build-recipe.json`: source/compiler/recipe record for the first build.
- `portable-notegen.cdx.json`: CycloneDX locked dependency inventory.
- `portable-build-provenance.json`: actual portable binary hashes and source pins.
- `installer-manifest.json`: verified workflow/artifact identities, reproduced
  executable hashes, installer acceptance and installer payload inventory.
- `portable-metadata.sha256`: hashes of the published metadata files.
- The portable ZIP and its SHA256 file, alongside the tested installer.

The installer workflow independently checks that the comparison's source and two
executable hashes match the actual installer payload before building the installer.
The installer adds CRT libraries; those are recorded separately in its manifest.

## Native build attestations

On eligible repositories, the trusted main-branch workflow uses a commit-pinned
`actions/attest` action to produce GitHub/Sigstore build provenance for the portable
ZIP and comparison report. The attestation job has OIDC/attestation permissions;
the jobs compiling project code do not. Pull-request builds never publish these
attestations. Verify an available portable build attestation with GitHub CLI:

```powershell
gh attestation verify NoteGen-Rust-Windows-x64.zip --repo InduwaraLasindu/NoteGen
```

Check the reported repository, workflow, commit and subject digest against the
release manifest. A checksum or unsigned JSON file is not a cryptographic
attestation. The Windows Authenticode signing feature remains excluded.

[GitHub's availability rules](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations)
require Enterprise Cloud for private/internal build attestations. Public
repositories use the supported default. An eligible private repository can set
`ENABLE_PRIVATE_ATTESTATIONS=true`; a configured attestation failure fails the
workflow. Otherwise `attestation-support.json` explicitly records that the native
service was not configured. No signed build attestation is claimed in that case.

The compare logic is tested twice with matching synthetic byte fixtures, corrupt
and missing binaries, changed source, recipe, dependency and environment records.
Those logic tests do not substitute for the independent real compilation gate.
