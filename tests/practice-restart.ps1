param([string]$Profile)
$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
# One throwaway coordinator for the whole script (tests/coordinator.cjs).
if(-not$env:MCOLLAB_TEST_URL){
 $forward=@($PSBoundParameters.GetEnumerator()|ForEach-Object{"-$($_.Key)";$_.Value})
 node tests/coordinator.cjs -- powershell -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath @forward
 exit $LASTEXITCODE
}
if(-not$Profile){
 $Profile=Join-Path $PWD ('.tools/practice-smoke-'+[guid]::NewGuid().ToString('N'))
 New-Item -ItemType Directory -Path $Profile|Out-Null
}
$profileRoot=(Resolve-Path -LiteralPath $Profile).Path
$toolsRoot=(Resolve-Path -LiteralPath '.tools').Path
if(-not$profileRoot.StartsWith($toolsRoot+[IO.Path]::DirectorySeparatorChar)-or-not(Split-Path -Leaf $profileRoot).StartsWith('practice-smoke-')){throw 'Refusing a non-isolated profile'}
$binary=Join-Path $PWD 'target/debug/miliastra-collab.exe'
$previousRoot=$env:MCOLLAB_TEST_DATA_DIR;$previousArgs=$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS;$previousPhase=$env:MCOLLAB_RESTART_PHASE
$process=$null
try{
 $env:MCOLLAB_TEST_DATA_DIR=$profileRoot;$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=19224'
 foreach($phase in @('prepare','resume')){
  $env:MCOLLAB_RESTART_PHASE=$phase
  $process=Start-Process -FilePath $binary -ArgumentList '--headless-smoke' -WindowStyle Hidden -PassThru
  $ready=$false
  for($i=0;$i-lt40;$i++){try{Invoke-RestMethod 'http://127.0.0.1:19224/json/version'|Out-Null;$ready=$true;break}catch{Start-Sleep -Milliseconds 250}}
  if(-not$ready){throw 'Native restart test WebView did not start'}
  node tests/practice-restart.cjs
  if($LASTEXITCODE-ne0){throw 'Practice restart check failed'}
  if(-not$process.HasExited-and$process.Path-eq$binary){Stop-Process -Id $process.Id;$process.WaitForExit()}
  # Wait for this test's WebView debug port to close.
  for($i=0;$i-lt40;$i++){try{Invoke-RestMethod 'http://127.0.0.1:19224/json/version'|Out-Null;Start-Sleep -Milliseconds 100}catch{break}}
 }
}finally{
 if($process-and-not$process.HasExited-and$process.Path-eq$binary){Stop-Process -Id $process.Id}
 $env:MCOLLAB_TEST_DATA_DIR=$previousRoot;$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=$previousArgs;$env:MCOLLAB_RESTART_PHASE=$previousPhase
}
