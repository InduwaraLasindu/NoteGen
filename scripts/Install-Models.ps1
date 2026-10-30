[CmdletBinding()]
param([switch]$MaximumAccuracy,[string[]]$ModelIds=@(),[string]$Root=(Split-Path $PSScriptRoot -Parent))
$ErrorActionPreference='Stop'
$Root=(Resolve-Path $Root).Path
Push-Location $Root
try{
 $cli=Join-Path $Root 'notegen-cli.exe'
 $catalogPath=Join-Path $Root 'assets/model-downloads.json'
 if(-not(Test-Path $catalogPath)){$catalogPath=Join-Path $Root 'rust/downloads.json'}
 $catalog=Get-Content $catalogPath -Raw | ConvertFrom-Json
 $settings=Join-Path $env:TEMP ('notegen-'+[guid]::NewGuid().ToString()+'.json')
 & $cli defaults -o $settings
 if($LASTEXITCODE -ne 0){throw 'Could not create adaptive settings'}
 try{
  $s=Get-Content $settings -Raw | ConvertFrom-Json
  if($MaximumAccuracy){$s.preset='maximum'}
  [IO.File]::WriteAllText($settings,($s|ConvertTo-Json -Depth 10),[Text.UTF8Encoding]::new($false))
  $planJson=& $cli --settings $settings plan --catalog
  if($LASTEXITCODE -ne 0){throw 'Hardware planning failed'}
  $plan=$planJson | ConvertFrom-Json
  if($ModelIds.Count -eq 0){$ModelIds=@($plan.models | ForEach-Object id)}
  if($ModelIds.Count -eq 0){throw 'No model fits available memory. Free memory and retry.'}
  Write-Host "Recommended for this PC: $($ModelIds -join ', '). Downloads happen only during setup."
  foreach($id in $ModelIds){
   $m=$catalog | Where-Object id -EQ $id | Select-Object -First 1
   if(-not $m){throw "No pinned download for $id; register compatible GGUF files in Advanced."}
   $dir=Join-Path $Root "models/$id"
   New-Item -ItemType Directory -Force $dir | Out-Null
   $paths=@()
   foreach($f in @($m.weights,$m.projector)){
    $path=Join-Path $dir $f.file
    if(-not(Test-Path $path) -or (Get-FileHash $path -Algorithm SHA256).Hash -ne $f.sha256){
     $part="$path.part"
     & curl.exe --fail --location --retry 2 --continue-at - $f.url --output $part
     if($LASTEXITCODE -ne 0){throw "Download failed: $($f.file). Re-run to resume."}
     if((Get-FileHash $part -Algorithm SHA256).Hash -ne $f.sha256){throw "Checksum mismatch: $($f.file). Not registered."}
     Move-Item $part $path -Force
    }
    $paths+=$path
   }
   & $cli register --id $id --weights $paths[0] --projector $paths[1]
   if($LASTEXITCODE -ne 0){throw "Registration failed: $id"}
  }
 }finally{Remove-Item $settings -ErrorAction SilentlyContinue}
 Write-Host 'Models ready. Start NoteGen.exe and choose your quality preset.'
}finally{Pop-Location}
