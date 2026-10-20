param([Parameter(Mandatory=$true)][string]$StagePath)
$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
# One throwaway coordinator for the whole script (tests/coordinator.cjs).
if(-not$env:MCOLLAB_TEST_URL){
 $forward=@($PSBoundParameters.GetEnumerator()|ForEach-Object{"-$($_.Key)";$_.Value})
 node tests/coordinator.cjs --port=18787 -- powershell -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath @forward
 exit $LASTEXITCODE
}
$stageFixture=(Resolve-Path -LiteralPath $StagePath).Path
$binary=Join-Path $PWD 'target/debug/miliastra-collab.exe'
if (-not (Test-Path -LiteralPath $binary)) { throw 'Build the debug desktop app first.' }
$previousRoot=$env:MCOLLAB_TEST_DATA_DIR
$previousStage=$env:MCOLLAB_SMOKE_STAGE
$previousArguments=$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
$process=$null
try {
 $env:MCOLLAB_TEST_DATA_DIR=Join-Path $PWD ('.tools/native-smoke-'+[guid]::NewGuid().ToString('N'))
 $env:MCOLLAB_SMOKE_STAGE=$stageFixture
 $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=19223'
 $process=Start-Process -FilePath $binary -ArgumentList '--headless-smoke' -WindowStyle Hidden -PassThru
 $ready=$false
 for ($attempt=0;$attempt -lt 40;$attempt++) {
  try { Invoke-RestMethod 'http://127.0.0.1:19223/json/version' | Out-Null; $ready=$true; break } catch { Start-Sleep -Milliseconds 250 }
 }
 if (-not $ready) { throw 'Native test WebView did not start.' }
 node tests/native-smoke.cjs
 $result=$LASTEXITCODE
} finally {
 if ($process -and -not $process.HasExited -and $process.Path -eq $binary) { Stop-Process -Id $process.Id }
 $env:MCOLLAB_TEST_DATA_DIR=$previousRoot
 $env:MCOLLAB_SMOKE_STAGE=$previousStage
 $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=$previousArguments
}
exit $result
