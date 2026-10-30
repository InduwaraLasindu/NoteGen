[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $repoRoot
$python = Join-Path $repoRoot '.venv\Scripts\python.exe'
if (-not (Test-Path -LiteralPath $python)) { $python = 'python' }
# Both tests and lint are build gates. A failure prevents packaging.
& $python -m ruff check src tests scripts
if ($LASTEXITCODE -ne 0) { throw 'Lint gate failed.' }
$env:QT_QPA_PLATFORM = 'offscreen'
& $python -m pytest -q --junitxml=build/test-results.xml
if ($LASTEXITCODE -ne 0) { throw 'Test gate failed.' }
Remove-Item Env:QT_QPA_PLATFORM -ErrorAction SilentlyContinue
& $python -m PyInstaller --noconfirm --clean --windowed --name NoteGen --paths src `
    --collect-all pypdfium2 --collect-all pypdfium2_raw --collect-all docx `
    --exclude-module PySide6.QtWebEngineCore --exclude-module torch `
    scripts/gui_entry.py
if ($LASTEXITCODE -ne 0) { throw 'Desktop build failed.' }
& $python -m PyInstaller --noconfirm --clean --console --name NoteGenCLI --paths src `
    --collect-all pypdfium2 --collect-all pypdfium2_raw --collect-all docx --exclude-module PySide6 `
    scripts/cli_entry.py
if ($LASTEXITCODE -ne 0) { throw 'CLI build failed.' }
Copy-Item LICENSE dist\NoteGen\LICENSE.txt
Copy-Item THIRD_PARTY_NOTICES.md dist\NoteGen\THIRD_PARTY_NOTICES.md
Copy-Item docs\WINDOWS.md dist\NoteGen\START-HERE.md
Copy-Item scripts\Start-LocalAI.ps1 dist\NoteGen\Start-LocalAI.ps1
& $python scripts\copy_licenses.py dist\NoteGen\licenses
if ($LASTEXITCODE -ne 0) { throw 'Dependency license collection failed.' }
Copy-Item -Recurse dist\NoteGenCLI dist\NoteGen\CLI -Force
Compress-Archive -Path dist\NoteGen -DestinationPath dist\NoteGen-Windows-x64.zip -Force
Write-Host 'Built dist\NoteGen-Windows-x64.zip'
