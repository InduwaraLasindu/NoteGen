#Requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Installer,[string]$Version='0.4.1',[string]$Evidence='build/installer-evidence',[string]$PreviousInstaller='')
$ErrorActionPreference='Stop'
$Installer=(Resolve-Path $Installer).Path
New-Item -ItemType Directory -Force $Evidence | Out-Null
$Evidence=(Resolve-Path $Evidence).Path
$installDir=Join-Path $env:LOCALAPPDATA ('Programs/NoteGen installer QA '+[guid]::NewGuid().ToString('N'))
$registry='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{6D37E156-3C8D-4BB7-AAC8-8F6C4C8F331A}_is1'
$results=[System.Collections.Generic.List[string]]::new()
function Wait-QAProcess([Diagnostics.Process]$process,[string]$stage,[int]$seconds=180,[string]$LogPath='') {
 Write-Host "[QA] Waiting for $stage (PID $($process.Id), limit $seconds seconds)"
 $watch=[Diagnostics.Stopwatch]::StartNew();$nextLog=0
 while($true){
  $process.Refresh()
  if($process.HasExited){break}
  if($watch.Elapsed.TotalSeconds -ge $nextLog){
   Write-Host "[QA] $stage active: $([int]$watch.Elapsed.TotalSeconds)s"
   if($LogPath -and (Test-Path $LogPath)){Get-Content $LogPath -Tail 8 -ErrorAction SilentlyContinue | ForEach-Object {Write-Host "[installer] $_"}}
   $nextLog=$watch.Elapsed.TotalSeconds+15
  }
  if($watch.Elapsed.TotalSeconds -gt $seconds){
   if($LogPath -and (Test-Path $LogPath)){Get-Content $LogPath -Tail 80 -ErrorAction SilentlyContinue | Write-Host}
   # Stop the named disposable parent without a blocking descendant-tree walk.
   Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
   throw "$stage exceeded $seconds seconds. The disposable test parent was stopped; see the streamed installer log."
  }
  Start-Sleep -Milliseconds 250
 }
 $code=$process.ExitCode
 Write-Host "[QA] $stage exited with code $code"
 return $code
}
function Install-NoteGen([string]$name,[switch]$AI) {
 Write-Host "[QA] Installing: $name"
 $installArgs=@('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-','/TASKS=desktopicon',('/DIR="{0}"' -f $installDir),('/LOG="{0}"' -f (Join-Path $Evidence "$name.log")))
 if($AI){$installArgs+=@('/AI=1','/AIBACKEND=cpu')}
 $p=Start-Process -FilePath $Installer -ArgumentList $installArgs -PassThru
 $exit=Wait-QAProcess $p "Installer $name" $(if($AI){600}else{180}) (Join-Path $Evidence "$name.log")
 if($exit -ne 0){throw "Installer failed ($name): $exit"}
}
function Test-Desktop([string]$name) {
 Write-Host "[QA] Launching desktop: $name"
 $p=Start-Process -FilePath (Join-Path $installDir 'NoteGen.exe') -WorkingDirectory $env:TEMP -PassThru
 try {
  Start-Sleep -Seconds 5
  $p.Refresh()
  if($p.HasExited){throw "Installed desktop exited ($name): $($p.ExitCode)"}
  $results.Add("Installed GUI launched: $name")
 } finally {if(-not $p.HasExited){Stop-Process -Id $p.Id -Force}}
}
try {
 if($PreviousInstaller){
  $newInstaller=$Installer
  $Installer=(Resolve-Path $PreviousInstaller).Path
  Install-NoteGen 'previous-0.4-install'
 Write-Host "[QA] Previous installation: CLI version"
  $previousVersion=& (Join-Path $installDir 'notegen-cli.exe') --version
  if($LASTEXITCODE -ne 0 -or $previousVersion -notmatch '0\.4\.0'){throw 'Previous installer did not contain NoteGen 0.4.0.'}
  Test-Desktop 'previous 0.4 installation'
  New-Item -ItemType Directory -Force (Join-Path $installDir 'user-data') | Out-Null
  $legacyMarker=Join-Path $installDir 'user-data/legacy-0.4-document.txt'
  [IO.File]::WriteAllText($legacyMarker,'EXISTING 0.4 DATA MUST SURVIVE')
  $Installer=$newInstaller
 }
 Install-NoteGen 'install'
 if($PreviousInstaller -and [IO.File]::ReadAllText($legacyMarker) -ne 'EXISTING 0.4 DATA MUST SURVIVE'){throw 'Version 0.4.0 to 0.4.1 upgrade changed user data.'}
 if($PreviousInstaller){$results.Add('Real 0.4.0 to 0.4.1 upgrade preserved existing user data and updated the executable')}

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
 Write-Host "[QA] Built-in PowerShell AI setup dry run"
 & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $installDir 'scripts/Configure-AI.ps1') -DryRun -NoPause
 if($LASTEXITCODE -ne 0){throw 'Start menu AI setup helper failed validation.'}
 Test-Desktop 'fresh installation'
 Push-Location $installDir
 try {
 Write-Host "[QA] Installed CLI and native dependencies"
  $reported=& ./notegen-cli.exe --version
  if($LASTEXITCODE -ne 0 -or $reported -notmatch [regex]::Escape($Version)){throw 'Installed executable version does not match the release.'}
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
 Write-Host "[QA] PDF rasterization, OCR and editable table export"
  & ./notegen-cli.exe convert $scan --preset fast -o $scanWord
  if($LASTEXITCODE -ne 0){throw 'Installed PDF rendering/OCR failed.'}
  $archive=[IO.Compression.ZipFile]::OpenRead($scanWord)
  try {
   $reader=[IO.StreamReader]::new($archive.GetEntry('word/document.xml').Open())
   try{$xml=$reader.ReadToEnd()}finally{$reader.Dispose()}
   if($xml -notmatch '1250' -or $xml -notmatch '<w:tbl>'){throw 'Installed OCR/table export assertion failed.'}
  } finally {$archive.Dispose()}
 Write-Host "[QA] Writer profile and confirmed glyph"
  $writer=(& ./notegen-cli.exe add-writer 'Installer QA writer') | ConvertFrom-Json
  if($LASTEXITCODE -ne 0 -or -not $writer.id){throw 'Installed writer creation failed.'}
  Add-Type -AssemblyName System.Drawing
  $glyph=Join-Path $Evidence 'glyph-r.png'
  $bitmap=[Drawing.Bitmap]::new(140,160);$graphics=[Drawing.Graphics]::FromImage($bitmap)
  $typeface=[Drawing.Font]::new('Arial',100,[Drawing.FontStyle]::Regular,[Drawing.GraphicsUnit]::Pixel)
  try{$graphics.Clear([Drawing.Color]::White);$graphics.DrawString('r',$typeface,[Drawing.Brushes]::Black,10,10);$bitmap.Save($glyph,[Drawing.Imaging.ImageFormat]::Png)}finally{$typeface.Dispose();$graphics.Dispose();$bitmap.Dispose()}
  & ./notegen-cli.exe teach $writer.id $glyph r --glyph
  if($LASTEXITCODE -ne 0){throw 'Installed confirmed handwriting learning failed.'}
  & ./notegen-cli.exe export-writer $writer.id -o (Join-Path $Evidence 'installed-writer.zip')
  if($LASTEXITCODE -ne 0){throw 'Installed profile export failed.'}
  $fontPack=Join-Path $Evidence 'installed-font.zip'
 Write-Host "[QA] TrueType font export"
  & ./notegen-cli.exe font-pack $writer.id -o $fontPack --family 'Installer QA handwriting'
  if($LASTEXITCODE -ne 0){throw 'Installed font pack export failed.'}
  $fontArchive=[IO.Compression.ZipFile]::OpenRead($fontPack)
  try{
   $fontFile=Join-Path $Evidence 'installed-handwriting.ttf'
   [IO.Compression.ZipFileExtensions]::ExtractToFile($fontArchive.GetEntry('Handwriting.ttf'),$fontFile,$true)
  }finally{$fontArchive.Dispose()}
 Write-Host "[QA] Windows native font rendering"
  & (Join-Path $PSScriptRoot 'Test-Font.ps1') -Font $fontFile -Preview (Join-Path $Evidence 'installed-font.png')
  if(-not $?){throw 'Windows rejected the installed-app font export.'}
  $writerFile=Join-Path $installDir "user-data/writers/$($writer.id)/profile.json"
  $writerHash=(Get-FileHash $writerFile -Algorithm SHA256).Hash
 } finally {Pop-Location}
 $results.Add('Installed CLI PDF rasterization, real OCR, editable Word table and native dependencies passed')
 $results.Add('Installed writer creation, confirmed crop learning, profile/font export and native Windows TTF rendering passed')
 Write-Host '[QA] One-click AI setup through the actual installer'
 Install-NoteGen 'integrated-ai-setup' -AI
 $aiLog=Get-Content (Join-Path $Evidence 'integrated-ai-setup.log') -Raw
 if($aiLog -notmatch 'NOTEGEN_AI_SETUP_VERIFIED' -or $aiLog -notmatch 'NGPROGRESS\|1000\|'){throw 'Installer did not finish its integrated AI progress/setup branch.'}
 $aiProfile=Join-Path $installDir 'ai-settings.json'
 $ai=Get-Content $aiProfile -Raw | ConvertFrom-Json
 Copy-Item -LiteralPath $aiProfile -Destination (Join-Path $Evidence 'installed-ai-settings.json') -Force
 $activeServer=[string]$ai.server
 if(-not [IO.Path]::IsPathRooted($activeServer)){$activeServer=Join-Path $installDir $activeServer}
 Write-Host "[QA] Active AI runtime: $activeServer"
 # Rust canonical paths can use the Windows extended prefix; ? is a wildcard to Test-Path.
 if(-not [IO.File]::Exists($activeServer)){throw "Installer AI runtime path is missing: $activeServer"}
 Push-Location $installDir
 try {
  & ./notegen-cli.exe --settings ./ai-settings.json plan | Out-File (Join-Path $Evidence 'installed-ai-plan.json') -Encoding utf8
  if($LASTEXITCODE -ne 0){throw 'Installed AI settings could not produce a valid native plan.'}
 } finally {Pop-Location}
 Test-Desktop 'after integrated AI setup'
 $registrations=@(Get-ChildItem (Join-Path $installDir 'models') -File -Filter '*.notegen.json')
 $families=@($registrations | ForEach-Object {(Get-Content $_ -Raw | ConvertFrom-Json).family} | Sort-Object -Unique)
 if($families.Count -lt 3){throw "Installer Maximum setup expected 3 independent fitting AI families, got $($families.Count)."}
 Copy-Item $aiProfile (Join-Path $Evidence 'installed-ai-settings.json')
 $results.Add('Actual installer one-click AI setup downloaded, checksum verified and activated a CPU runtime and three independent fitting model families with live progress')
 Push-Location $installDir
 try {
  $cancel=Join-Path $Evidence 'cancel-ai';[IO.File]::WriteAllText($cancel,'cancel')
  & ./notegen-cli.exe --settings ./ai-settings.json --cancel-file $cancel --installer-progress setup --models-only --output ./cancelled-ai-settings.json 2>&1 | Out-File (Join-Path $Evidence 'ai-cancel.log')
  if($LASTEXITCODE -eq 0 -or (Test-Path './cancelled-ai-settings.json')){throw 'Cancelled setup unexpectedly succeeded.'}
  if((Get-Content (Join-Path $Evidence 'ai-cancel.log') -Raw) -notmatch 'NG900'){throw 'Setup cancellation did not return its explicit cancellation code.'}
 } finally {Pop-Location}
 $results.Add('Native AI setup cancellation preserved the active runtime/model installation')
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
 Write-Host "[QA] Reinstall and runtime/data preservation"
 Install-NoteGen 'upgrade'
 if((Get-FileHash $writerFile -Algorithm SHA256).Hash -ne $writerHash){throw 'Upgrade modified a handwriting profile.'}
 if([IO.File]::ReadAllText($model) -ne 'USER MODEL MUST SURVIVE' -or [IO.File]::ReadAllText($document) -ne 'USER DOCUMENT MUST SURVIVE'){throw 'Upgrade modified user data.'}
 if((Get-FileHash $engine -Algorithm SHA256).Hash -ne $customEngineHash){throw 'Upgrade overwrote the selected AI runtime.'}
 Copy-Item $engineBackup $engine -Force
 Remove-Item $engineBackup
 Test-Desktop 'upgrade'
 $results.Add('Upgrade/reinstall preserved models, document data and the selected AI runtime')
 Write-Host "[QA] Uninstall and data retention"
 $uninstall=Start-Process -FilePath (Join-Path $installDir 'unins000.exe') -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',('/LOG="{0}"' -f (Join-Path $Evidence 'uninstall.log'))) -PassThru
 $exit=Wait-QAProcess $uninstall 'Uninstaller'
 if($exit -ne 0){throw "Uninstall failed: $exit"}
 for($i=0;$i -lt 20 -and (Test-Path (Join-Path $installDir 'NoteGen.exe'));$i++){Start-Sleep -Milliseconds 250}
 if(Test-Path (Join-Path $installDir 'NoteGen.exe')){throw 'Uninstall left the installed EXE behind.'}
 if(Test-Path $registry){throw 'Uninstall left its Apps & Features registration behind.'}
 if(Test-Path (Join-Path $programs 'NoteGen/NoteGen.lnk')){throw 'Uninstall left the Start menu shortcut behind.'}
 if(Test-Path (Join-Path $desktop 'NoteGen.lnk')){throw 'Uninstall left the desktop shortcut behind.'}
 if([IO.File]::ReadAllText($model) -ne 'USER MODEL MUST SURVIVE' -or [IO.File]::ReadAllText($document) -ne 'USER DOCUMENT MUST SURVIVE'){throw 'Uninstall deleted user data.'}
 if((Get-FileHash $writerFile -Algorithm SHA256).Hash -ne $writerHash){throw 'Uninstall deleted a handwriting profile.'}
 $results.Add('Uninstall removed the app and shortcuts while preserving user data and handwriting profiles')
 $results | Set-Content (Join-Path $Evidence 'passed.txt') -Encoding utf8
 Write-Host ($results -join "`n")
} finally {
 # Cleanup is restricted to this disposable CI installation, never a user's real install.
 if($installDir -and (Test-Path $installDir)){Remove-Item $installDir -Recurse -Force -ErrorAction SilentlyContinue}
}
