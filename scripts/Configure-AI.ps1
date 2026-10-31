[CmdletBinding()]
param(
 [ValidateSet('Auto','CUDA13','CUDA12','Vulkan','CPU')][string]$Backend='Auto',
 [switch]$Balanced,
 [switch]$DryRun,
 [switch]$NoPause,
 [string]$Root=''
)
$ErrorActionPreference='Stop'
$setupFailed=$false
$transcriptStarted=$false
try {
 $scriptFolder=Split-Path -Parent $MyInvocation.MyCommand.Path
 if([string]::IsNullOrWhiteSpace($Root)){$Root=Split-Path $scriptFolder -Parent}
 $Root=(Resolve-Path $Root).Path
 foreach($file in @('notegen-cli.exe','scripts/Setup-Runtime.ps1','scripts/Install-Models.ps1','assets/model-downloads.json')) {
  if(-not(Test-Path (Join-Path $Root $file))){throw "Installation file missing: $file. Run the NoteGen installer again."}
 }
 if($DryRun){Write-Host 'AI setup entry point validated; no downloads requested.'; return}
 $active=Get-Process -Name 'llama-server' -ErrorAction SilentlyContinue | Where-Object {
  try{$_.Path -and $_.Path.StartsWith((Join-Path $Root 'runtime'),[StringComparison]::OrdinalIgnoreCase)}catch{$false}
 }
 if($active){throw 'Finish or cancel running NoteGen conversions, then run AI setup again.'}
 $logs=Join-Path $Root 'logs'
 New-Item -ItemType Directory -Force $logs | Out-Null
 $log=Join-Path $logs ('ai-setup-'+(Get-Date -Format 'yyyyMMdd-HHmmss')+'.log')
 Start-Transcript -Path $log | Out-Null
 $transcriptStarted=$true
 Write-Host 'NoteGen local AI setup' -ForegroundColor Cyan
 Write-Host 'Runtime and model downloads happen only during setup. Conversion stays local.'
 Write-Host 'Models are selected for the available RAM/VRAM. Downloads may require several GB.'
 Write-Host "Setup log: $log"
 & (Join-Path $scriptFolder 'Setup-Runtime.ps1') -Root $Root -Backend $Backend
 & (Join-Path $scriptFolder 'Install-Models.ps1') -Root $Root -MaximumAccuracy:(-not $Balanced)
 Write-Host 'AI setup complete. Restart NoteGen to refresh the installed readers.' -ForegroundColor Green
} catch {
 $setupFailed=$true
 Write-Host "Setup failed: $($_.Exception.Message)" -ForegroundColor Red
 Write-Host 'For an older NVIDIA driver, retry with -Backend CUDA12 or -Backend Vulkan.'
 Write-Host 'Re-running setup resumes partial downloads and verifies SHA256 before registration.'
} finally {
 if($transcriptStarted){Stop-Transcript | Out-Null}
 if(-not $NoPause -and -not $DryRun){Read-Host 'Press Enter to close' | Out-Null}
}
if($setupFailed){exit 1}
