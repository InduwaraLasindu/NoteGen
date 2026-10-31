#Requires -Version 7.0
[CmdletBinding()]
param([switch]$SkipBuild)
$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$sourceRoot=(Get-Location).Path
if(-not $SkipBuild){
 & cargo build --locked --release --features desktop --bins -j 2
 if($LASTEXITCODE -ne 0){throw 'Rust release compilation failed'}
}
$stage=Join-Path (Get-Location) ('build/rust-package-'+[guid]::NewGuid().ToString())
$package=Join-Path $stage 'NoteGen'
New-Item -ItemType Directory -Force $package,"$package/runtime","$package/runtime/tesseract","$package/scripts","$package/licenses","$package/assets","$package/docs",dist | Out-Null
Copy-Item target/release/NoteGen.exe,target/release/notegen-cli.exe $package
Copy-Item scripts/Setup-Runtime.ps1,scripts/Install-Models.ps1 "$package/scripts"
Copy-Item rust/downloads.json "$package/assets/model-downloads.json"
Copy-Item README.md,LICENSE,THIRD_PARTY_NOTICES.md $package
Copy-Item docs/WINDOWS.md,docs/TESTING.md,docs/RUST_PLAN.md,docs/STUDIO.md "$package/docs"
$native=Join-Path (Get-Location) 'build/pdfium-rust'
if(-not(Test-Path "$native/bin/pdfium.dll")){
 New-Item -ItemType Directory -Force $native | Out-Null
 & curl.exe --fail --location --retry 2 'https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/8125/pdfium-win-x64.tgz' -o "$native/pdfium.tgz"
 if($LASTEXITCODE -ne 0){throw 'PDFium download failed'}
 & tar -xzf "$native/pdfium.tgz" -C $native
 if($LASTEXITCODE -ne 0){throw 'PDFium extraction failed'}
}
Copy-Item "$native/bin/pdfium.dll" "$package/runtime"
Get-ChildItem $native -File | Where-Object Name -Match 'LICENSE|COPYING|NOTICE' | Copy-Item -Destination "$package/licenses"
if(Test-Path runtime/llama-server.exe){
 Get-ChildItem runtime -File | Where-Object {$_.Extension -in @('.dll','.exe') -or $_.Name -match 'LICENSE|COPYING|NOTICE'} | Copy-Item -Destination "$package/runtime" -Force
}
$tess=Join-Path $env:ProgramFiles 'Tesseract-OCR'
if(-not(Test-Path "$tess/tesseract.exe")){$tess=(Get-Command tesseract.exe -ErrorAction Stop).Source | Split-Path -Parent}
if(-not(Test-Path "$tess/tessdata")){throw 'Tesseract installation folder must contain tessdata; a command shim cannot be bundled.'}
Copy-Item "$tess/*" "$package/runtime/tesseract" -Recurse -Force
Get-ChildItem "$package/runtime/tesseract/tessdata" -Filter '*.traineddata' | Where-Object Name -NotIn @('eng.traineddata','osd.traineddata') | Remove-Item
$metadataJson=& cargo metadata --locked --all-features --format-version 1
if($LASTEXITCODE -ne 0){throw 'Cargo metadata failed'}
$metadata=$metadataJson | ConvertFrom-Json -AsHashtable
$records=foreach($p in $metadata.packages){
 if($p.source){
  $destination="$package/licenses/$($p.name)-$($p.version)"
  New-Item -ItemType Directory -Force $destination | Out-Null
  $base=Split-Path $p.manifest_path -Parent
  foreach($f in Get-ChildItem $base -Recurse -File | Where-Object Name -Match '^(LICENSE|LICENCE|COPYING|NOTICE|OFL)([-._].*)?$'){
   $relative=$f.FullName.Substring($base.Length).TrimStart('\','/')
   $target=Join-Path $destination $relative
   New-Item -ItemType Directory -Force (Split-Path $target -Parent) | Out-Null
   Copy-Item $f.FullName $target
  }
 }
 "$($p.name) $($p.version) | $($p.license) | $($p.repository)"
}
$records | Set-Content "$package/licenses/RUST-DEPENDENCIES.txt" -Encoding utf8
Push-Location $package
try{
 & ./notegen-cli.exe self-test -o ./package-test.docx
 if($LASTEXITCODE -ne 0){throw 'Packaged Word export failed'}
 Remove-Item ./package-test.docx
 $fixture=Join-Path $sourceRoot 'build/native-fixture/scan.pdf'
 if(Test-Path $fixture){
  & ./notegen-cli.exe convert $fixture --preset fast -o ./package-scan.docx
  if($LASTEXITCODE -ne 0){throw 'Packaged scanned-PDF conversion failed'}
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $doc=[IO.Compression.ZipFile]::OpenRead((Join-Path (Get-Location) 'package-scan.docx'))
  try{
   $reader=[IO.StreamReader]::new($doc.GetEntry('word/document.xml').Open())
   try{$text=$reader.ReadToEnd()}finally{$reader.Dispose()}
   if($text -notmatch '1250' -or $text -notmatch '<w:tbl>'){throw 'Packaged OCR/table assertion failed'}
  }finally{$doc.Dispose()}
  Remove-Item ./package-scan.docx
 }
 & ./runtime/tesseract/tesseract.exe --version
 if($LASTEXITCODE -ne 0){throw 'Packaged Tesseract failed'}
}finally{Pop-Location}
$zip=Join-Path (Get-Location) 'dist/NoteGen-Rust-Windows-x64.zip'
Compress-Archive -Path $package -DestinationPath $zip -Force
(Get-FileHash $zip -Algorithm SHA256).Hash.ToLower() | Set-Content "$zip.sha256"
Write-Host "Ready: $zip"

