[CmdletBinding()]
param([ValidateSet('Auto','CUDA13','CUDA12','Vulkan','CPU')][string]$Backend='Auto',[string]$Root=(Split-Path $PSScriptRoot -Parent))
$ErrorActionPreference='Stop'
$Root=(Resolve-Path $Root).Path
if ($Backend -eq 'Auto') {$Backend=if(Get-Command nvidia-smi.exe -ErrorAction SilentlyContinue){'CUDA13'}else{'Vulkan'}}
$assets=@{
 CUDA13=@(@{name='llama-b12032-bin-win-cuda-13.4-x64.zip';sha='4b2d4c4678192fbadc443e2c406fef06356d1b064ae9ab79b9f67e4329903511'},@{name='cudart-llama-bin-win-cuda-13.4-x64.zip';sha='738f8c251ac22b70c3ae6f83a10cf222725df0395246a2cf58f32bdb85fbe668'})
 CUDA12=@(@{name='llama-b12032-bin-win-cuda-12.4-x64.zip';sha='36c7e24ad048cceabf00c1b49d0f0411b954c6a410a7ff4300af8721a94fd00a'},@{name='cudart-llama-bin-win-cuda-12.4-x64.zip';sha='8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6'})
 Vulkan=@(@{name='llama-b12032-bin-win-vulkan-x64.zip';sha='19a8bbf269fa683ae7bc04f886c9c2a93e0655c2606f37d2b60069bf9e8bcc32'})
 CPU=@(@{name='llama-b12032-bin-win-cpu-x64.zip';sha='5296f5d18bb7d422c7c26f4bb9bc796d9d4f41e1312a8c296cff475bdc60d0d4'})
}
$runtime=Join-Path $Root 'runtime'
$cache=Join-Path $runtime 'downloads'
New-Item -ItemType Directory -Force $cache,$runtime | Out-Null
foreach($asset in $assets[$Backend]){
 $name=$asset.name;$archive=Join-Path $cache $name
 if(-not(Test-Path $archive) -or (Get-FileHash $archive -Algorithm SHA256).Hash -ne $asset.sha){
  $part="$archive.part"
  & curl.exe --fail --location --retry 2 --continue-at - "https://github.com/ggml-org/llama.cpp/releases/download/b12032/$name" --output $part
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
