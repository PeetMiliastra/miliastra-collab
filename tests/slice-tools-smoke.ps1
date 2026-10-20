param([string]$BinaryPath='target/debug/miliastra-collab.exe')
$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
# One throwaway coordinator for the whole script (tests/coordinator.cjs).
if(-not$env:MCOLLAB_TEST_URL){
 $forward=@($PSBoundParameters.GetEnumerator()|ForEach-Object{"-$($_.Key)";$_.Value})
 node tests/coordinator.cjs -- powershell -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath @forward
 exit $LASTEXITCODE
}
$binary=(Resolve-Path -LiteralPath $BinaryPath).Path
$profileRoot=Join-Path $PWD ('.tools/slice-tools-native-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $profileRoot|Out-Null
[IO.File]::WriteAllText((Join-Path $profileRoot 'coordination.json'),'{"serviceUrl":"http://127.0.0.1:19738"}',(New-Object Text.UTF8Encoding($false)))
$previousWebview=$env:WEBVIEW2_USER_DATA_FOLDER;$previousRoot=$env:MCOLLAB_TEST_DATA_DIR;$previousArgs=$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
$process=$null
try{
 $env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $profileRoot 'webview';$env:MCOLLAB_TEST_DATA_DIR=$profileRoot;$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=19238'
 $name='privacy'+[guid]::NewGuid().ToString('N').Substring(0,16)
 $existing=Get-Process -Name 'miliastra-collab' -ErrorAction SilentlyContinue | Where-Object {$_.Path -eq $binary}
 if($existing){throw 'The isolated test binary is already running; refusing to share its profile.'}
 $process=Start-Process -FilePath $binary -ArgumentList '--headless-smoke' -WindowStyle Hidden -PassThru
 $ready=$false
 for($attempt=0;$attempt -lt 80;$attempt++){try{Invoke-RestMethod 'http://127.0.0.1:19238/json/version' -TimeoutSec 1|Out-Null;$ready=$true;break}catch{Start-Sleep -Milliseconds 250}}
 if(-not$ready){throw 'Private test WebView did not start.'}
 node tests/slice-tools-native.cjs
 if($LASTEXITCODE -ne 0){throw 'Native slice tools check failed.'}
 Write-Output ("Preserved synthetic evidence: "+$profileRoot)
}finally{
 if($process -and -not$process.HasExited -and $process.Path -eq $binary){Stop-Process -Id $process.Id}
 $env:WEBVIEW2_USER_DATA_FOLDER=$previousWebview;$env:MCOLLAB_TEST_DATA_DIR=$previousRoot;$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=$previousArgs
}
