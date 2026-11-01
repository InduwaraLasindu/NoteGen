#Requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Installer,[string]$Version='0.3.0',[string]$Evidence='build/installer-evidence')
$ErrorActionPreference='Stop'
$Installer=(Resolve-Path $Installer).Path
New-Item -ItemType Directory -Force $Evidence | Out-Null
$Evidence=(Resolve-Path $Evidence).Path
$installDir=Join-Path $env:LOCALAPPDATA ('Programs/NoteGen installer QA '+[guid]::NewGuid().ToString('N'))
$registry='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{6D37E156-3C8D-4BB7-AAC8-8F6C4C8F331A}_is1'
$results=[System.Collections.Generic.List[string]]::new()
function Install-NoteGen([string]$name) {
 $installArgs=@('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/TASKS=desktopicon',('/DIR="{0}"' -f $installDir),('/LOG="{0}"' -f (Join-Path $Evidence "$name.log")))
 $p=Start-Process -FilePath $Installer -ArgumentList $installArgs -PassThru -Wait
 if($p.ExitCode -ne 0){throw "Installer failed ($name): $($p.ExitCode)"}
}
function Test-Desktop([string]$name) {
 $p=Start-Process -FilePath (Join-Path $installDir 'NoteGen.exe') -WorkingDirectory $env:TEMP -PassThru
 try {
  Start-Sleep -Seconds 5
  $p.Refresh()
  if($p.HasExited){throw "Installed desktop exited ($name): $($p.ExitCode)"}
  $results.Add("Installed GUI launched: $name")
 } finally {if(-not $p.HasExited){Stop-Process -Id $p.Id -Force}}
}
try {
 Install-NoteGen 'install'
 foreach($f in @('NoteGen.exe','notegen-cli.exe','runtime/pdfium.dll','runtime/tesseract/tesseract.exe','runtime/llama-server.exe','vcruntime140.dll','runtime/vcruntime140.dll','scripts/Configure-AI.ps1','unins000.exe')) {
  if(-not(Test-Path (Join-Path $installDir $f))){throw "Installed dependency missing: $f"}
 }
 $entry=Get-ItemProperty $registry -ErrorAction Stop
 if($entry.DisplayVersion -ne $Version -or $entry.InstallLocation.TrimEnd('\') -ne $installDir.TrimEnd('\')){throw 'Apps & Features registration is incorrect.'}
 $programs=[Environment]::GetFolderPath('Programs')
 $desktop=[Environment]::GetFolderPath('DesktopDirectory')
 foreach($p in @((Join-Path $programs 'NoteGen/NoteGen.lnk'),(Join-Path $programs 'NoteGen/Set up local AI.lnk'),(Join-Path $desktop 'NoteGen.lnk'))) {
  if(-not(Test-Path $p)){throw "Shortcut missing: $p"}
 }
 $results.Add('Install, dependencies, per-user registration and shortcuts passed')
 # The shipped setup entry point must work in built-in Windows PowerShell, with no downloads.
 & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $installDir 'scripts/Configure-AI.ps1') -DryRun -NoPause
 if($LASTEXITCODE -ne 0){throw 'Start menu AI setup helper failed validation.'}
 Test-Desktop 'fresh installation'
 Push-Location $installDir
 try {
  & ./notegen-cli.exe self-test -o (Join-Path $Evidence 'installed-word.docx')
  if($LASTEXITCODE -ne 0){throw 'Installed CLI Word export failed.'}
  & ./runtime/tesseract/tesseract.exe --version
  if($LASTEXITCODE -ne 0){throw 'Installed Tesseract failed.'}
  # The conversion pipeline rasterizes this fixture before OCR; it never extracts PDF text.
  $content="2 w 40 300 400 120 re S 240 300 m 240 420 l S 40 360 m 440 360 l S`nBT /F1 24 Tf 1 0 0 1 40 510 Tm (NoteGen installer test) Tj ET`nBT /F1 16 Tf 1 0 0 1 60 385 Tm (Item) Tj 1 0 0 1 260 385 Tm (Amount) Tj 1 0 0 1 60 325 Tm (Sales) Tj 1 0 0 1 260 325 Tm (1250) Tj ET`n"
  $objects=@('<< /Type /Catalog /Pages 2 0 R >>','<< /Type /Pages /Kids [3 0 R] /Count 1 >>','<< /Type /Page /Parent 2 0 R /MediaBox [0 0 480 576] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>','<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',("<< /Length $($content.Length) >>`nstream`n$content"+'endstream'))
  $pdf="%PDF-1.4`n";$offsets=[System.Collections.Generic.List[int]]::new()
  for($i=0;$i -lt $objects.Count;$i++){$offsets.Add($pdf.Length);$pdf+="$($i+1) 0 obj`n$($objects[$i])`nendobj`n"}
  $xref=$pdf.Length;$pdf+="xref`n0 6`n0000000000 65535 f `n"
  foreach($offset in $offsets){$pdf+=$offset.ToString('D10')+" 00000 n `n"}
  $pdf+="trailer`n<< /Size 6 /Root 1 0 R >>`nstartxref`n$xref`n%%EOF`n"
  $scan=Join-Path $Evidence 'installer-scan.pdf'
  [IO.File]::WriteAllText($scan,$pdf,[Text.Encoding]::ASCII)
  $scanWord=Join-Path $Evidence 'installed-scan.docx'
  & ./notegen-cli.exe convert $scan --preset fast -o $scanWord
  if($LASTEXITCODE -ne 0){throw 'Installed PDF rendering/OCR failed.'}
  $archive=[IO.Compression.ZipFile]::OpenRead($scanWord)
  try {
   $reader=[IO.StreamReader]::new($archive.GetEntry('word/document.xml').Open())
   try{$xml=$reader.ReadToEnd()}finally{$reader.Dispose()}
   if($xml -notmatch '1250' -or $xml -notmatch '<w:tbl>'){throw 'Installed OCR/table export assertion failed.'}
  } finally {$archive.Dispose()}
 } finally {Pop-Location}
 $results.Add('Installed CLI PDF rasterization, real OCR, editable Word table and native dependencies passed')
 New-Item -ItemType Directory -Force (Join-Path $installDir 'models'),(Join-Path $installDir 'user-data') | Out-Null
 $model=Join-Path $installDir 'models/keep-user-model.marker'
 $document=Join-Path $installDir 'user-data/keep-user-document.txt'
 [IO.File]::WriteAllText($model,'USER MODEL MUST SURVIVE')
 [IO.File]::WriteAllText($document,'USER DOCUMENT MUST SURVIVE')
 $engine=Join-Path $installDir 'runtime/llama-server.exe'
 $engineBackup=Join-Path $Evidence 'original-llama-server.exe'
 Copy-Item $engine $engineBackup
 # A different valid PE stands in for a user-selected CUDA/Vulkan server during upgrade.
 Copy-Item (Join-Path $installDir 'notegen-cli.exe') $engine -Force
 $customEngineHash=(Get-FileHash $engine -Algorithm SHA256).Hash
 Install-NoteGen 'upgrade'
 if([IO.File]::ReadAllText($model) -ne 'USER MODEL MUST SURVIVE' -or [IO.File]::ReadAllText($document) -ne 'USER DOCUMENT MUST SURVIVE'){throw 'Upgrade modified user data.'}
 if((Get-FileHash $engine -Algorithm SHA256).Hash -ne $customEngineHash){throw 'Upgrade overwrote the selected AI runtime.'}
 Copy-Item $engineBackup $engine -Force
 Remove-Item $engineBackup
 Test-Desktop 'upgrade'
 $results.Add('Upgrade/reinstall preserved models, document data and the selected AI runtime')
 $uninstall=Start-Process -FilePath (Join-Path $installDir 'unins000.exe') -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',('/LOG="{0}"' -f (Join-Path $Evidence 'uninstall.log'))) -PassThru -Wait
 if($uninstall.ExitCode -ne 0){throw "Uninstall failed: $($uninstall.ExitCode)"}
 for($i=0;$i -lt 20 -and (Test-Path (Join-Path $installDir 'NoteGen.exe'));$i++){Start-Sleep -Milliseconds 250}
 if(Test-Path (Join-Path $installDir 'NoteGen.exe')){throw 'Uninstall left the installed EXE behind.'}
 if(Test-Path $registry){throw 'Uninstall left its Apps & Features registration behind.'}
 if(Test-Path (Join-Path $programs 'NoteGen/NoteGen.lnk')){throw 'Uninstall left the Start menu shortcut behind.'}
 if(Test-Path (Join-Path $desktop 'NoteGen.lnk')){throw 'Uninstall left the desktop shortcut behind.'}
 if([IO.File]::ReadAllText($model) -ne 'USER MODEL MUST SURVIVE' -or [IO.File]::ReadAllText($document) -ne 'USER DOCUMENT MUST SURVIVE'){throw 'Uninstall deleted user data.'}
 $results.Add('Uninstall removed the app and shortcuts while preserving user data')
 $results | Set-Content (Join-Path $Evidence 'passed.txt') -Encoding utf8
 Write-Host ($results -join "`n")
} finally {
 # Cleanup is restricted to this disposable CI installation, never a user's real install.
 if($installDir -and (Test-Path $installDir)){Remove-Item $installDir -Recurse -Force -ErrorAction SilentlyContinue}
}
