# Real Windows inference and installer acceptance; not used by the application.
[CmdletBinding()]
param()
$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$sourceRoot=(Get-Location).Path
$smoke=Join-Path $sourceRoot 'build/ai-smoke/NoteGen'
New-Item -ItemType Directory -Force $smoke,"$smoke/assets","$smoke/runtime" | Out-Null
Copy-Item target/release/notegen-cli.exe $smoke
Copy-Item rust/downloads.json "$smoke/assets/model-downloads.json"
Get-ChildItem runtime -File | Where-Object {$_.Extension -in @('.dll','.exe')} | Copy-Item -Destination "$smoke/runtime"
& ./scripts/Install-Models.ps1 -Root $smoke -ModelIds glm-ocr,lighton3-1b,qwen35-2b
if($LASTEXITCODE -ne 0){throw 'Actual model installation failed'}
$env:NOTEGEN_LIVE_SERVER="$smoke/runtime/llama-server.exe"
foreach($id in @('glm-ocr','lighton3-1b','qwen35-2b')){
 $pin=Get-Content "$smoke/assets/model-downloads.json" -Raw | ConvertFrom-Json | Where-Object id -EQ $id
 $env:NOTEGEN_LIVE_ID=$id
 $env:NOTEGEN_LIVE_ROOT="$smoke/results/$id"
 $env:NOTEGEN_LIVE_WEIGHTS="$smoke/models/$id/$($pin.weights.file)"
 $env:NOTEGEN_LIVE_PROJECTOR="$smoke/models/$id/$($pin.projector.file)"
 & cargo test --locked --features desktop --test engine live_model -j 2 -- --ignored --nocapture
 if($LASTEXITCODE -ne 0){throw "Real local inference failed: $id"}
}
# Exercise actual model-major orchestration and persisted evidence, not only adapters.
$settings=Join-Path $smoke 'smoke-settings.json'
Push-Location $smoke
try{
 & ./notegen-cli.exe check-update glm-ocr | Out-File './publisher-update-metadata.json' -Encoding utf8
 if($LASTEXITCODE -ne 0){throw 'Real publisher update metadata validation failed'}
 # Exercise the third-party managed-reader registration through the real native pipeline.
 $custom=Get-Content './models/glm-ocr.notegen.json' -Raw | ConvertFrom-Json -AsHashtable
 $custom.id='custom-glm';$custom.name='Custom local GLM reader';$custom.rank=300;$custom.alias='stale-service-alias'
 [IO.File]::WriteAllText((Join-Path $smoke 'custom-reader.json'),($custom|ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))
 & ./notegen-cli.exe register-reader './custom-reader.json'
 if($LASTEXITCODE -ne 0){throw 'Actual custom GGUF registration failed'}
 $registered=Get-Content './models/custom-glm.notegen.json' -Raw | ConvertFrom-Json
 if($registered.alias){throw 'Managed reader retained an incompatible external service alias'}

 & ./notegen-cli.exe defaults -o $settings
 if($LASTEXITCODE -ne 0){throw 'CLI defaults failed'}
 $s=Get-Content $settings -Raw | ConvertFrom-Json
 $s.preset='maximum';$s.backend='cpu';$s.handwriting=$true;$s.crop_retries=1
 $s.dpi=150;$s.max_tokens=2048;$s.request_timeout_secs=600;$s.auto_rotate=$false
 $s.pdfium="$sourceRoot/build/pdfium-rust/bin/pdfium.dll"
 [IO.File]::WriteAllText($settings,($s|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
 & ./notegen-cli.exe --settings $settings convert "$sourceRoot/build/native-fixture/scan.pdf" --preset maximum --handwriting -o ./maximum-smoke.docx
 if($LASTEXITCODE -ne 0){throw 'Full Maximum Accuracy conversion failed'}
 $project=Get-ChildItem .notegen -Recurse -Filter project.json | Select-Object -First 1
 $page=Get-Content (Join-Path $project.DirectoryName 'page-00001.json') -Raw | ConvertFrom-Json
 $readers=@($page.candidates | Where-Object view -EQ 'full' | ForEach-Object family | Sort-Object -Unique)
 if($readers.Count -lt 4){throw "Expected Tesseract plus three native AI families; got $($readers -join ',')"}
}finally{Pop-Location}

