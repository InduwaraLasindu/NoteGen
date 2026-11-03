#Requires -Version 7.0
[CmdletBinding()]
param([string]$TargetDirectory = 'target')
$ErrorActionPreference = 'Stop'
$sourceRoot = (Resolve-Path -LiteralPath (Split-Path $PSScriptRoot -Parent)).Path
if (-not $IsWindows) { throw 'This recipe builds the Windows MSVC release.' }
$keys = @('CARGO_ENCODED_RUSTFLAGS', 'RUSTFLAGS', 'CARGO_TARGET_DIR', 'SOURCE_DATE_EPOCH', 'CARGO_INCREMENTAL')
$previous = @{}
foreach ($key in $keys) { $previous[$key] = [Environment]::GetEnvironmentVariable($key, 'Process') }
Push-Location $sourceRoot
try {
    if ($env:RUSTFLAGS -or $env:CARGO_ENCODED_RUSTFLAGS) { throw 'Use a clean build environment; ambient Rust flags are not part of this recipe.' }
    $commit = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $commit -notmatch '^[a-f0-9]{40}$') { throw 'A complete source checkout is required.' }
    $dirty = @(& git status --porcelain --untracked-files=normal)
    if ($LASTEXITCODE -ne 0 -or $dirty.Count -ne 0) { throw 'Build provenance requires a clean source checkout.' }
    $epoch = (& git show -s --format=%ct HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $epoch -notmatch '^\d+$') { throw 'Source timestamp is unavailable.' }
    $compiler = (& rustc +1.99.1 -vV) -join "`n"
    if ($LASTEXITCODE -ne 0 -or $compiler -notmatch '(?m)^host: x86_64-pc-windows-msvc$') { throw 'Pinned Rust 1.99.1 with the x64 MSVC host is required.' }
    $target = [IO.Path]::GetFullPath($TargetDirectory, $sourceRoot)
    $cargoRoot = if ($env:CARGO_HOME) { [IO.Path]::GetFullPath($env:CARGO_HOME) } else { Join-Path ([Environment]::GetFolderPath('UserProfile')) '.cargo' }
    $rustupRoot = if ($env:RUSTUP_HOME) { [IO.Path]::GetFullPath($env:RUSTUP_HOME) } else { Join-Path ([Environment]::GetFolderPath('UserProfile')) '.rustup' }
    $flags = @(('--remap-path-prefix='+$sourceRoot+'=/notegen'), ('--remap-path-prefix='+$cargoRoot+'=/cargo'), ('--remap-path-prefix='+$rustupRoot+'=/rustup'), ('--remap-path-prefix='+$target+'=/build'), '-C', 'link-arg=/Brepro')
    $env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]31
    $env:CARGO_TARGET_DIR = $target
    $env:SOURCE_DATE_EPOCH = $epoch
    $env:CARGO_INCREMENTAL = '0'
    & cargo +1.99.1 build --locked --release --features desktop --bins -j 2
    if ($LASTEXITCODE -ne 0) { throw 'Deterministic release compilation failed.' }
    $record = [ordered]@{
        schema = 1
        source_commit = $commit
        source_date_epoch = $epoch
        rustc = $compiler
        cargo_lock_sha256 = (Get-FileHash -LiteralPath Cargo.lock -Algorithm SHA256).Hash.ToLowerInvariant()
        recipe_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
        target = 'x86_64-pc-windows-msvc'
        profile = 'release; desktop; bins; locked; incremental=0; path-remapping; /Brepro'
        runner_image = $env:ImageOS
        runner_image_version = $env:ImageVersion
        ci_repository = $env:GITHUB_REPOSITORY
        ci_run = $env:GITHUB_RUN_ID
        ci_job = $env:GITHUB_JOB
        subjects = @(foreach ($name in @('NoteGen.exe', 'notegen-cli.exe')) {
            $file = Join-Path $target "release/$name"
            @{name=$name;sha256=(Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant();bytes=(Get-Item -LiteralPath $file).Length}
        })
        note = 'One build record. Reproducibility is established only by comparing independently compiled executable bytes, not by this recipe alone. Third-party bundled runtimes are outside the comparison.'
    }
    $record | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $target 'release/reproducible-build.json') -Encoding utf8NoBOM
} finally {
    Pop-Location
    foreach ($key in $keys) { [Environment]::SetEnvironmentVariable($key, $previous[$key], 'Process') }
}
