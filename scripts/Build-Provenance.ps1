#Requires -Version 7.0
[CmdletBinding()]
param(
    [string]$OutputDirectory = 'dist/provenance',
    [string]$BinaryDirectory = 'target/release',
    [string]$SourceCommit = $env:GITHUB_SHA
)
$ErrorActionPreference = 'Stop'
$sourceRoot = Split-Path $PSScriptRoot -Parent
Push-Location $sourceRoot
try {
    if (-not $SourceCommit) {
        $SourceCommit = & git rev-parse HEAD
        if ($LASTEXITCODE -ne 0) { throw 'Provide -SourceCommit when building outside a Git checkout.' }
    }
    $SourceCommit = ([string]$SourceCommit).Trim().ToLowerInvariant()
    if ($SourceCommit -notmatch '^(?:[a-f0-9]{40}|[a-f0-9]{64})$') { throw 'Source commit must be a full hexadecimal Git object ID.' }
    $metadataText = & cargo metadata --locked --all-features --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Locked dependency metadata failed.' }
    $metadata = ($metadataText -join "`n") | ConvertFrom-Json -AsHashtable
    $rootPackage = $metadata.packages | Where-Object { $_.name -eq 'notegen' -and -not $_.source } | Select-Object -First 1
    if (-not $rootPackage) { throw 'NoteGen package missing from dependency metadata.' }
    $checksums = @{}
    foreach ($section in ((Get-Content -LiteralPath Cargo.lock -Raw) -split '\[\[package\]\]')) {
        $packageName = [regex]::Match($section, '(?m)^name = "([^"]+)"').Groups[1].Value
        $packageVersion = [regex]::Match($section, '(?m)^version = "([^"]+)"').Groups[1].Value
        $checksum = [regex]::Match($section, '(?m)^checksum = "([a-f0-9]{64})"').Groups[1].Value
        if ($checksum) { $checksums["$packageName@$packageVersion"] = $checksum }
    }
    $refs = @{}
    foreach ($package in $metadata.packages) {
        $refs[$package.id] = "pkg:cargo/$($package.name)@$($package.version)"
    }
    $components = @(foreach ($package in ($metadata.packages | Where-Object source | Sort-Object name, version)) {
        $component = [ordered]@{
            type = 'library'
            'bom-ref' = $refs[$package.id]
            name = $package.name
            version = $package.version
            purl = $refs[$package.id]
        }
        if ($package.license) { $component.licenses = @(@{ expression = $package.license }) }
        $checksum = $checksums["$($package.name)@$($package.version)"]
        if ($checksum) { $component.hashes = @(@{ alg = 'SHA-256'; content = $checksum }) }
        $component
    })
    $dependencies = @(foreach ($node in ($metadata.resolve.nodes | Sort-Object id)) {
        if ($refs.ContainsKey($node.id)) {
            @{ ref = $refs[$node.id]; dependsOn = @($node.dependencies | ForEach-Object { $refs[$_] } | Where-Object { $_ } | Sort-Object -Unique) }
        }
    })
    $bom = [ordered]@{
        bomFormat = 'CycloneDX'
        specVersion = '1.6'
        version = 1
        metadata = @{ component = @{ type = 'application'; 'bom-ref' = $refs[$rootPackage.id]; name = 'NoteGen'; version = $rootPackage.version; purl = $refs[$rootPackage.id] } }
        components = $components
        dependencies = $dependencies
    }
    $binaryRoot = (Resolve-Path -LiteralPath $BinaryDirectory).Path
    $binaries = @(Get-ChildItem -LiteralPath $binaryRoot -File -Recurse | Where-Object { $_.Extension -in @('.exe', '.dll') -or $_.Name -in @('NoteGen', 'notegen-cli') } | Sort-Object FullName)
    if ($binaries.Count -eq 0) { throw 'Build provenance needs compiled NoteGen binaries.' }
    $subjects = @(foreach ($binary in $binaries) {
        $relative = [IO.Path]::GetRelativePath($binaryRoot, $binary.FullName).Replace('\', '/')
        @{ name = $relative; digest = @{ sha256 = (Get-FileHash -LiteralPath $binary.FullName -Algorithm SHA256).Hash.ToLowerInvariant() } }
    })
    $pins = @(foreach ($relative in @('Cargo.lock', 'rust/catalog.json', 'rust/downloads.json', 'rust/runtime-downloads.json')) {
        @{ name = $relative; sha256 = (Get-FileHash -LiteralPath $relative -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    $rustVersion = (& rustc --version) -join ''
    if ($LASTEXITCODE -ne 0) { throw 'Cannot identify Rust compiler.' }
    $cargoVersion = (& cargo --version) -join ''
    if ($LASTEXITCODE -ne 0) { throw 'Cannot identify Cargo.' }
    $provenance = [ordered]@{
        schema_version = 1
        application = @{ name = 'NoteGen'; version = $rootPackage.version }
        source = @{ repository = 'https://github.com/InduwaraLasindu/NoteGen'; commit = $SourceCommit }
        build_tools = @{ rustc = $rustVersion; cargo = $cargoVersion }
        inputs = $pins
        subjects = $subjects
        claims = @('Records all-feature locked dependency inventory, including target-specific packages, and actual binary SHA256 hashes.', 'This file is unsigned; use the separate GitHub artifact attestation to verify CI identity.', 'Byte-for-byte reproducibility is not asserted.')
    }
    # Metadata's local manifest paths, environment variables, usernames and document data are deliberately omitted.
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    $inventoryPath = Join-Path $OutputDirectory 'notegen.cdx.json'
    $provenancePath = Join-Path $OutputDirectory 'build-provenance.json'
    $bom | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $inventoryPath -Encoding utf8NoBOM
    $provenance | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $provenancePath -Encoding utf8NoBOM
    $hashRecords = @('notegen.cdx.json', 'build-provenance.json') | ForEach-Object {
        "$( (Get-FileHash -LiteralPath (Join-Path $OutputDirectory $_) -Algorithm SHA256).Hash.ToLowerInvariant() )  $_"
    }
    $hashRecords | Set-Content -LiteralPath (Join-Path $OutputDirectory 'provenance.sha256') -Encoding utf8NoBOM
    Write-Host "Generated dependency inventory ($($components.Count) components), binary provenance and SHA256 manifest."
} finally {
    Pop-Location
}
