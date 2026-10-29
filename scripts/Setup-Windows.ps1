# One-time package installation; no document is opened or uploaded.
[CmdletBinding()]
param([switch]$Launch)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $repoRoot
$python = Join-Path $repoRoot '.venv\Scripts\python.exe'
if (-not (Test-Path -LiteralPath $python)) {
    & py -3.12 -m venv .venv
    if ($LASTEXITCODE -ne 0) { throw 'Install Python 3.12 x64 from python.org, with the Python launcher.' }
}
& $python -m pip install -r (Join-Path $repoRoot 'requirements-lock.txt')
if ($LASTEXITCODE -ne 0) { throw 'Package installation failed. Check the console output.' }
& $python -m pip install --no-deps --no-build-isolation -e $repoRoot
if ($LASTEXITCODE -ne 0) { throw 'NoteGen installation failed.' }
Write-Host 'NoteGen is ready. Tesseract and the CUDA vision runtime/model are installed separately.'
Write-Host 'Run .\scripts\Run-NoteGen.ps1 to open the app.'
if ($Launch) { & $python -m notegen.app }
