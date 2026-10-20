param([string]$BinaryPath='.tools/review2-target/debug/miliastra-collab.exe',[int]$Cycles=12,[int]$Seed=1729,[switch]$Build,[switch]$SkipLeaseExpiry)
$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if($Build -or -not(Test-Path -LiteralPath $BinaryPath)){
 . ./scripts/environment.ps1
 npm.cmd run build
 if($LASTEXITCODE -ne 0){throw 'Frontend build failed'}
 New-Item -ItemType Directory -Force -Path (Join-Path $PWD '.tools') | Out-Null
 $config=Join-Path $PWD '.tools/reliability-build.json'
 [IO.File]::WriteAllText($config,'{"identifier":"dev.miliastra.collab.reliability","build":{"beforeBuildCommand":""}}',(New-Object Text.UTF8Encoding($false)))
 $env:CARGO_TARGET_DIR=Join-Path $PWD '.tools/review2-target'
 $env:CARGO_PROFILE_DEV_DEBUG='0'
 npm.cmd run tauri build -- --debug --no-bundle --config $config
 if($LASTEXITCODE -ne 0){throw 'Native reliability build failed'}
 $BinaryPath=Join-Path $env:CARGO_TARGET_DIR 'debug/miliastra-collab.exe'
}
$binary=(Resolve-Path -LiteralPath $BinaryPath).Path
node tests/reliability/run.cjs "--binary=$binary" "--cycles=$Cycles" "--seed=$Seed" "--skip-lease-expiry=$($SkipLeaseExpiry.IsPresent.ToString().ToLowerInvariant())"
exit $LASTEXITCODE
