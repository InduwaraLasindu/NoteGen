#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ReferenceArchive,
    [Parameter(Mandatory)][string]$CandidateDirectory,
    [Parameter(Mandatory)][string]$SourceCommit,
    [string]$Output = 'build/reproducibility.json'
)
$ErrorActionPreference = 'Stop'
if ($SourceCommit -notmatch '^[a-f0-9]{40}$') { throw 'Expected exact source commit.' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $ReferenceArchive).Path)
try {
    function Read-Entry([string]$Name, [long]$Limit) {
        $matches = @($archive.Entries | Where-Object FullName -CEQ $Name)
        if ($matches.Count -ne 1 -or $matches[0].Length -gt $Limit) { throw "Missing, duplicate or oversized reference member: $Name" }
        return $matches[0]
    }
    $entry = Read-Entry 'NoteGen/provenance/reproducible-build.json' 100000
    $reader = [IO.StreamReader]::new($entry.Open())
    try { $reference = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
    $candidatePath = Join-Path $CandidateDirectory 'reproducible-build.json'
    if ((Get-Item -LiteralPath $candidatePath).Length -gt 100000) { throw 'Oversized candidate recipe record.' }
    $candidate = Get-Content -LiteralPath $candidatePath -Raw | ConvertFrom-Json
    foreach ($record in @($reference,$candidate)) {
        if ($record.schema -ne 1 -or $record.source_commit -cne $SourceCommit -or $record.target -cne 'x86_64-pc-windows-msvc') { throw 'Build record source/target mismatch.' }
    }
    foreach ($key in @('source_date_epoch','rustc','cargo_lock_sha256','recipe_sha256','target','profile','runner_image','runner_image_version')) {
        if (-not $reference.$key -or $reference.$key -cne $candidate.$key) { throw "Independent build environments differ: $key. No reproducibility assertion was made." }
    }
    $subjects = @(foreach ($name in @('NoteGen.exe','notegen-cli.exe')) {
        $entry = Read-Entry "NoteGen/$name" 250000000
        $stream = $entry.Open()
        try { $first = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)).ToLowerInvariant() } finally { $stream.Dispose() }
        $candidateFile = Join-Path $CandidateDirectory $name
        if ((Get-Item -LiteralPath $candidateFile).Length -gt 250000000) { throw 'Candidate binary exceeds budget.' }
        $second = (Get-FileHash -LiteralPath $candidateFile -Algorithm SHA256).Hash.ToLowerInvariant()
        foreach ($record in @($reference,$candidate)) {
            $declared = @($record.subjects | Where-Object name -CEQ $name)
            $actual = if ($record -eq $reference) { $first } else { $second }
            if ($declared.Count -ne 1 -or $declared[0].sha256 -cne $actual) { throw "Build record does not match actual bytes: $name" }
        }
        if ($first -cne $second) { throw "Independent executable hashes differ: $name. First=$first Second=$second" }
        @{name=$name;sha256=$first;bytes=(Get-Item -LiteralPath $candidateFile).Length}
    })
    $independent = $reference.ci_repository -and $reference.ci_repository -ceq $candidate.ci_repository -and $reference.ci_run -match '^\d+$' -and $reference.ci_run -ceq $candidate.ci_run -and $reference.ci_job -and $candidate.ci_job -and $reference.ci_job -cne $candidate.ci_job
    $report = [ordered]@{
        schema = 1
        source_commit = $SourceCommit
        cargo_lock_sha256 = $reference.cargo_lock_sha256
        recipe_sha256 = $reference.recipe_sha256
        target = $reference.target
        rustc = $reference.rustc
        runner_image = $reference.runner_image
        runner_image_version = $reference.runner_image_version
        subjects = $subjects
        matched = $true
        independent_jobs = [bool]$independent
        comparison = if($independent){'Exact SHA256 of two executable builds in different jobs of the same GitHub verification run; candidate target directory starts empty.'}else{'The supplied executable bytes match. Independent CI jobs were not established by their build records.'}
        scope = 'NoteGen.exe and notegen-cli.exe only. ZIP timestamps, installer, third-party binaries and different toolchain/SDK/runner versions are not asserted reproducible.'
        verification_run = "https://github.com/$env:GITHUB_REPOSITORY/actions/runs/$env:GITHUB_RUN_ID"
    }
    $parent = Split-Path ([IO.Path]::GetFullPath($Output)) -Parent
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Output -Encoding utf8NoBOM
    Write-Host "Provided executable bytes match. Independent CI jobs: $([bool]$independent)."
} finally { $archive.Dispose() }
