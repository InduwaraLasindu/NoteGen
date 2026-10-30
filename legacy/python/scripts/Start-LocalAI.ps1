# Local GGUFs only. Never downloads models or exposes the server to the LAN.
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$Server,
    [Parameter(Mandatory=$true)][string]$Model,
    [Parameter(Mandatory=$true)][string]$Projector,
    [ValidateRange(1024,65535)][int]$Port = 8080,
    [ValidateRange(8192,32768)][int]$ContextSize = 16384,
    [ValidateRange(512,4096)][int]$ImageTokens = 3072,
    [ValidateRange(1,128)][int]$GpuLayers = 99,
    [switch]$ProjectorOnCPU
)
$ErrorActionPreference = 'Stop'
$serverPath = (Resolve-Path -LiteralPath $Server).Path
$modelPath = (Resolve-Path -LiteralPath $Model).Path
$projectorPath = (Resolve-Path -LiteralPath $Projector).Path
if (-not $modelPath.EndsWith('.gguf') -or -not $projectorPath.EndsWith('.gguf')) {
    throw 'Choose a vision GGUF and its matching GGUF projector.'
}
$env:HF_HUB_OFFLINE = '1'
$env:LLAMA_ARG_OFFLINE = '1'
$serverArgs = @(
    '--model', $modelPath, '--mmproj', $projectorPath,
    '--host', '127.0.0.1', '--port', "$Port", '--alias', 'notegen-vision', '--offline',
    '--n-gpu-layers', "$GpuLayers", '--ctx-size', "$ContextSize", '--parallel', '1',
    '--flash-attn', 'on', '--batch-size', '512', '--ubatch-size', '128',
    '--image-max-tokens', "$ImageTokens", '--cache-ram', '0'
)
if ($ProjectorOnCPU) { $serverArgs += '--no-mmproj-offload' }
Write-Host "Local vision endpoint: http://127.0.0.1:$Port"
Write-Host 'Target: 32 GB RAM / 16 GB RTX. Actual peak memory depends on the model, scan and runtime.'
Write-Host 'If memory is tight, close other GPU apps, lower ImageTokens or use ProjectorOnCPU.'
Write-Host 'Keep this console open while NoteGen uses AI. Ctrl+C stops the local server.'
& $serverPath @serverArgs
exit $LASTEXITCODE
