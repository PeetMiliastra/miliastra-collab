$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
# One throwaway coordinator for the whole script (tests/coordinator.cjs).
if(-not$env:MCOLLAB_TEST_URL){
 $forward=@($PSBoundParameters.GetEnumerator()|ForEach-Object{"-$($_.Key)";$_.Value})
 node tests/coordinator.cjs -- powershell -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath @forward
 exit $LASTEXITCODE
}
$profileRoot=Join-Path $PWD ('.tools/invitation-smoke-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $profileRoot | Out-Null
$toolsRoot=(Resolve-Path -LiteralPath '.tools').Path
if(-not$profileRoot.StartsWith($toolsRoot+[IO.Path]::DirectorySeparatorChar)-or-not(Split-Path -Leaf $profileRoot).StartsWith('invitation-smoke-')){throw 'Refusing a non-isolated profile'}
$binary=Join-Path $PWD 'target/debug/miliastra-collab.exe'
$previousFolder=$env:MCOLLAB_TEST_FOLDER;$previousRoot=$env:MCOLLAB_TEST_DATA_DIR;$previousArgs=$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS;$previousPhase=$env:MCOLLAB_INVITATION_PHASE
$process=$null
$protocolKey='Registry::HKEY_CURRENT_USER\Software\Classes\miliastra-collab\shell\open\command'
$registeredBefore=if(Test-Path -LiteralPath $protocolKey){(Get-Item -LiteralPath $protocolKey).GetValue('')}else{$null}
try{
 $env:MCOLLAB_TEST_FOLDER=Join-Path $profileRoot 'exchange';$env:MCOLLAB_TEST_DATA_DIR=$profileRoot;$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=19229'
 foreach($phase in @('prepare','resume')){
  $env:MCOLLAB_INVITATION_PHASE=$phase
  $arguments=if($phase-eq'resume'){(Get-Content -LiteralPath (Join-Path $profileRoot 'invitation-checkpoint.json') -Raw | ConvertFrom-Json).protocol}else{'--headless-smoke'}
  $process=Start-Process -FilePath $binary -ArgumentList $arguments -WindowStyle Hidden -PassThru
  $ready=$false
  for($i=0;$i-lt40;$i++){try{Invoke-RestMethod 'http://127.0.0.1:19229/json/version'|Out-Null;$ready=$true;break}catch{Start-Sleep -Milliseconds 250}}
  if(-not$ready){throw 'Native restart test WebView did not start'}
  node tests/invitation-native.cjs
  if($LASTEXITCODE-ne0){throw 'Invitation protocol check failed'}
  if(-not$process.HasExited-and$process.Path-eq$binary){Stop-Process -Id $process.Id;$process.WaitForExit()}
  # Wait for this test's WebView debug port to close.
  for($i=0;$i-lt40;$i++){try{Invoke-RestMethod 'http://127.0.0.1:19229/json/version'|Out-Null;Start-Sleep -Milliseconds 100}catch{break}}
 }
 $registeredAfter=if(Test-Path -LiteralPath $protocolKey){(Get-Item -LiteralPath $protocolKey).GetValue('')}else{$null}
 if($registeredBefore-ne$registeredAfter){throw 'Isolated test changed the real protocol registration'}
}finally{
 if($process-and-not$process.HasExited-and$process.Path-eq$binary){Stop-Process -Id $process.Id}
 $env:MCOLLAB_TEST_FOLDER=$previousFolder;$env:MCOLLAB_TEST_DATA_DIR=$previousRoot;$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=$previousArgs;$env:MCOLLAB_INVITATION_PHASE=$previousPhase
}
