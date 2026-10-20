$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
# One throwaway coordinator for the whole script (tests/coordinator.cjs).
if(-not$env:MCOLLAB_TEST_URL){
 $forward=@($PSBoundParameters.GetEnumerator()|ForEach-Object{"-$($_.Key)";$_.Value})
 node tests/coordinator.cjs -- powershell -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath @forward
 exit $LASTEXITCODE
}
$binary=Join-Path $PWD 'target/debug/miliastra-collab.exe'
if(-not(Test-Path -LiteralPath $binary)){throw 'Build the isolated debug desktop package first.'}
$previousRoot=$env:MCOLLAB_TEST_DATA_DIR
$previousOverlay=$env:MCOLLAB_TEST_OVERLAY
$previousArguments=$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
$process=$null
try{
 $env:MCOLLAB_TEST_DATA_DIR=Join-Path $PWD ('.tools/practice-smoke-'+[guid]::NewGuid().ToString('N'))
 $env:MCOLLAB_TEST_OVERLAY='1'
 $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=19224'
 $process=Start-Process -FilePath $binary -ArgumentList '--headless-smoke' -WindowStyle Hidden -PassThru
 $ready=$false
 for($attempt=0;$attempt -lt 40;$attempt++){
  try{Invoke-RestMethod 'http://127.0.0.1:19224/json/version'|Out-Null;$ready=$true;break}catch{Start-Sleep -Milliseconds 250}
 }
 if(-not$ready){throw 'Practice test WebView did not start.'}
 node tests/practice-native.cjs
 $result=$LASTEXITCODE
}finally{
 if($process -and -not$process.HasExited -and $process.Path -eq $binary){Stop-Process -Id $process.Id}
 $env:MCOLLAB_TEST_DATA_DIR=$previousRoot
 $env:MCOLLAB_TEST_OVERLAY=$previousOverlay
 $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=$previousArguments
}
exit $result
