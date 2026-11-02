#Requires -Version 7.0
[CmdletBinding()]
param(
 [Parameter(Mandatory)][string]$Payload,
 [Parameter(Mandatory)][string]$Compiler,
 [string]$Version='0.5.0',
 [string]$Output='dist/installer'
)
$ErrorActionPreference='Stop'
if($Version -notmatch '^\d+\.\d+\.\d+$'){throw 'Use a numeric major.minor.patch version.'}
$sourceRoot=Split-Path $PSScriptRoot -Parent
$Payload=(Resolve-Path $Payload).Path
foreach($f in @('NoteGen.exe','notegen-cli.exe','runtime/pdfium.dll','runtime/llama-server.exe','runtime/tesseract/tesseract.exe','assets/model-downloads.json','provenance/build-provenance.json','provenance/notegen.cdx.json','provenance/provenance.sha256','LICENSE')) {
 if(-not(Test-Path (Join-Path $Payload $f))){throw "Verified payload is incomplete: $f"}
}
# The original package smoke test leaves only synthetic checkpoints. None belong in an installer.
foreach($p in @('.notegen','package-test.docx','package-scan.docx')) {
 $path=[IO.Path]::GetFullPath((Join-Path $Payload $p))
 if(-not $path.StartsWith($Payload+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'QA cleanup target escapes the explicit payload directory'}
 if(Test-Path -LiteralPath $path){
  if((Get-Item -LiteralPath $path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Refusing to clean a linked QA payload path'}
  Remove-Item -LiteralPath $path -Recurse -Force
 }
}
if(Test-Path (Join-Path $Payload 'models')){throw 'The installer payload must not contain downloaded model weights.'}
Copy-Item (Join-Path $sourceRoot 'README.md') $Payload -Force
Copy-Item (Join-Path $sourceRoot 'docs/STUDIO.md') (Join-Path $Payload 'docs') -Force
Copy-Item (Join-Path $sourceRoot 'docs/TESTING.md') (Join-Path $Payload 'docs') -Force
Copy-Item (Join-Path $PSScriptRoot 'Configure-AI.ps1') (Join-Path $Payload 'scripts') -Force
Copy-Item (Join-Path $sourceRoot 'docs/INSTALLER.md') (Join-Path $Payload 'docs') -Force
Get-Content (Join-Path $sourceRoot 'docs/INSTALLER.md') -Raw | Set-Content (Join-Path $Payload 'docs/INSTALLER.txt') -Encoding utf8
# App-local CRT files allow the per-user installation to start without developer tools.
$vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
if(-not(Test-Path $vswhere)){throw 'Visual Studio redistributable files were not found on this build machine.'}
$vs=& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if($LASTEXITCODE -ne 0 -or -not $vs){throw 'Visual C++ build tools/redist location could not be resolved.'}
$crt=Get-ChildItem (Join-Path $vs 'VC/Redist/MSVC') -Directory -Recurse -Filter 'Microsoft.VC*.CRT' |
 Where-Object {$_.FullName -match '[\\/]x64[\\/]'} | Sort-Object FullName -Descending | Select-Object -First 1
if(-not $crt){throw 'The x64 Visual C++ CRT redistributable folder is missing.'}
foreach($target in @($Payload,(Join-Path $Payload 'runtime'))) {
 Get-ChildItem $crt.FullName -File -Filter '*.dll' | Copy-Item -Destination $target -Force
}
@'
Microsoft Visual C++ runtime files accompany this application as app-local dependencies.
Copyright Microsoft Corporation. These files retain Microsoft's license terms.
Redistribution reference: https://learn.microsoft.com/cpp/windows/redistributing-visual-cpp-files
Distributable-code list: https://learn.microsoft.com/visualstudio/releases/2022/redistribution
'@ | Set-Content (Join-Path $Payload 'licenses/MICROSOFT-CRT.txt') -Encoding utf8
New-Item -ItemType Directory -Force $Output | Out-Null
$Output=(Resolve-Path $Output).Path
& $Compiler "/DAppVersion=$Version" "/DPayloadDir=$Payload" "/DOutputPath=$Output" (Join-Path $sourceRoot 'installer/NoteGen.iss')
if($LASTEXITCODE -ne 0){throw 'Inno Setup compilation failed.'}
$installer=Join-Path $Output "NoteGen-Setup-$Version.exe"
if(-not(Test-Path $installer)){throw 'The installer compiler did not produce the expected EXE.'}
$hash=(Get-FileHash $installer -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $([IO.Path]::GetFileName($installer))" | Set-Content "$installer.sha256" -Encoding utf8
Write-Host "Installer ready: $installer"
