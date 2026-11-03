#Requires -Version 7.0
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$workspace = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$root = Join-Path $workspace ('build/provenance-test-'+[guid]::NewGuid().ToString('N'))
$candidate = Join-Path $root 'candidate'
New-Item -ItemType Directory -Path $candidate -Force | Out-Null
try {
    $commit = 'a'*40
    $names = @('NoteGen.exe','notegen-cli.exe')
    foreach ($name in $names) { [IO.File]::WriteAllBytes((Join-Path $candidate $name), [Text.Encoding]::UTF8.GetBytes("SYNTHETIC COMPARISON FIXTURE $name")) }
    $record = @{schema=1;source_commit=$commit;target='x86_64-pc-windows-msvc';source_date_epoch='1';rustc='synthetic compiler fixture';cargo_lock_sha256=('b'*64);recipe_sha256=('c'*64);profile='fixture';runner_image='fixture';runner_image_version='fixture';subjects=@(foreach($name in $names){@{name=$name;sha256=(Get-FileHash -LiteralPath (Join-Path $candidate $name) -Algorithm SHA256).Hash.ToLowerInvariant()}})}
    $recordPath = Join-Path $candidate 'reproducible-build.json'
    $record | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $recordPath -Encoding utf8NoBOM
    $zipPath = Join-Path $root 'reference.zip'
    $zip = [IO.Compression.ZipFile]::Open($zipPath,[IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach($name in $names){[IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip,(Join-Path $candidate $name),"NoteGen/$name")|Out-Null}
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip,$recordPath,'NoteGen/provenance/reproducible-build.json')|Out-Null
    } finally { $zip.Dispose() }
    $compare = Join-Path $PSScriptRoot 'Compare-Reproducible.ps1'
    $report = Join-Path $root 'result.json'
    & $compare -ReferenceArchive $zipPath -CandidateDirectory $candidate -SourceCommit $commit -Output $report
    $value = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
    if (-not $value.matched -or $value.subjects.Count -ne 2 -or $value.independent_jobs) { throw 'Comparison fixture did not produce exact subjects or incorrectly claimed independent jobs.' }
    $first = Join-Path $candidate $names[0]
    $original = [IO.File]::ReadAllBytes($first)
    [IO.File]::AppendAllText($first,'CORRUPTED')
    $rejected = $false
    try { & $compare -ReferenceArchive $zipPath -CandidateDirectory $candidate -SourceCommit $commit -Output $report } catch { $rejected = $true }
    if (-not $rejected) { throw 'Corrupted executable was accepted.' }
    [IO.File]::WriteAllBytes($first,$original)
    foreach($key in @('source_commit','recipe_sha256','cargo_lock_sha256','runner_image_version')){
        $old = $record[$key]
        $record[$key] = 'changed'
        $record | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $recordPath -Encoding utf8NoBOM
        $rejected = $false
        try { & $compare -ReferenceArchive $zipPath -CandidateDirectory $candidate -SourceCommit $commit -Output $report } catch { $rejected = $true }
        if (-not $rejected) { throw "Changed build identity was accepted: $key" }
        $record[$key] = $old
    }
    $record | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $recordPath -Encoding utf8NoBOM
    Remove-Item -LiteralPath (Join-Path $candidate $names[1])
    $rejected = $false
    try { & $compare -ReferenceArchive $zipPath -CandidateDirectory $candidate -SourceCommit $commit -Output $report } catch { $rejected = $true }
    if (-not $rejected) { throw 'Missing candidate executable was accepted.' }
    Write-Host 'Build comparison contracts passed: exact bytes, corrupted bytes, changed source/recipe/dependencies/environment and missing executable.'
} finally {
    $resolved = [IO.Path]::GetFullPath($root)
    $allowed = [IO.Path]::GetFullPath((Join-Path $workspace 'build'))+[IO.Path]::DirectorySeparatorChar
    if (-not $resolved.StartsWith($allowed,[StringComparison]::OrdinalIgnoreCase)) { throw 'Refusing cleanup outside the test workspace.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
