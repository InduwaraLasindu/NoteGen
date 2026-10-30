[CmdletBinding()]
param([ValidateSet('Auto','CUDA13','CUDA12','Vulkan','CPU')][string]$Backend='Auto',[string]$Root=(Split-Path $PSScriptRoot -Parent))
$ErrorActionPreference='Stop'
$Root=(Resolve-Path $Root).Path
if ($Backend -eq 'Auto') {$Backend=if(Get-Command nvidia-smi.exe -ErrorAction SilentlyContinue){'CUDA13'}else{'Vulkan'}}
$assets=@{
 CUDA13=@(@{name='llama-b11998-bin-win-cuda-13.4-x64.zip';sha='ede53b39ce75578957d6ecba15a19dae723317313d39c051297aa73af13c4ce1'},@{name='cudart-llama-bin-win-cuda-13.4-x64.zip';sha='738f8c251ac22b70c3ae6f83a10cf222725df0395246a2cf58f32bdb85fbe668'})
 CUDA12=@(@{name='llama-b11998-bin-win-cuda-12.4-x64.zip';sha='a52b4f9de136e6fe245e2aa0c35b36d6886f79de3f71bfe04278b11b3113f49b'},@{name='cudart-llama-bin-win-cuda-12.4-x64.zip';sha='8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6'})
 Vulkan=@(@{name='llama-b11998-bin-win-vulkan-x64.zip';sha='47a32e3d164f8221658009d7f48794aed36ab9064791a5e04204a39d6eaf839a'})
 CPU=@(@{name='llama-b11998-bin-win-cpu-x64.zip';sha='2cdb7ff52769dff9bf19c4cf7d6b6e136e7b46249679d5d92ecd92631b8f7fb2'})
}
$runtime=Join-Path $Root 'runtime'
$cache=Join-Path $runtime 'downloads'
New-Item -ItemType Directory -Force $cache,$runtime | Out-Null
foreach($asset in $assets[$Backend]){
 $name=$asset.name;$archive=Join-Path $cache $name
 if(-not(Test-Path $archive) -or (Get-FileHash $archive -Algorithm SHA256).Hash -ne $asset.sha){
  $part="$archive.part"
  & curl.exe --fail --location --retry 2 --continue-at - "https://github.com/ggml-org/llama.cpp/releases/download/b11998/$name" --output $part
  if($LASTEXITCODE -ne 0){throw "Download failed: $name. Re-run to resume; remove only an invalid .part if its range is rejected."}
  if((Get-FileHash $part -Algorithm SHA256).Hash -ne $asset.sha){throw "SHA256 mismatch: $name. File was not installed."}
  Move-Item $part $archive -Force
 }
 $unpack=Join-Path $cache ([IO.Path]::GetFileNameWithoutExtension($name))
 Expand-Archive $archive $unpack -Force
 Get-ChildItem $unpack -Recurse -File | Where-Object {$_.Extension -eq '.dll' -or $_.Name -eq 'llama-server.exe'} | Copy-Item -Destination $runtime -Force
 Get-ChildItem $unpack -Recurse -File | Where-Object Name -Match 'LICENSE|COPYING|NOTICE' | ForEach-Object {Copy-Item $_.FullName (Join-Path $runtime "$([IO.Path]::GetFileNameWithoutExtension($name))-$($_.Name)") -Force}
}
& "$runtime/llama-server.exe" --list-devices
if($LASTEXITCODE -ne 0){throw 'Runtime did not start. Check NVIDIA driver compatibility or retry -Backend Vulkan (or CPU).'}
Write-Host "Runtime ready: $Backend. Next: ./scripts/Install-Models.ps1"
