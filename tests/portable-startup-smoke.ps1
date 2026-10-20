param([string]$BinaryPath='.tools/review2-target/debug/miliastra-collab.exe')
$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$binary=(Resolve-Path -LiteralPath $BinaryPath).Path
$profileRoot=Join-Path $PWD ('.tools/portable-startup-'+[guid]::NewGuid().ToString('N'))
$previousRoot=$env:MCOLLAB_TEST_DATA_DIR;$previousArgs=$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS;$previousHelper=$env:MCOLLAB_PORTABLE_HELPER
$process=$null
try {
 $env:MCOLLAB_TEST_DATA_DIR=$profileRoot;$env:MCOLLAB_PORTABLE_HELPER=$binary;$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=19245'
 python tests/portable-startup-profile.py prepare
 if($LASTEXITCODE -ne 0){throw 'Startup fixture preparation failed'}
 $journal=(Get-Content -LiteralPath (Join-Path $profileRoot 'journal-path.txt') -Raw).Trim()
 $existing=Get-Process -Name miliastra-collab -ErrorAction SilentlyContinue | Where-Object {$_.Path -eq $binary}
 if($existing){throw 'Isolated test binary already running'}
 $process=Start-Process -FilePath $binary -ArgumentList @('--headless-smoke','--portable-update-complete',('"{0}"' -f $journal)) -WindowStyle Hidden -PassThru
 $ready=$false
 for($attempt=0;$attempt -lt 80;$attempt++){try{Invoke-RestMethod 'http://127.0.0.1:19245/json/version' -TimeoutSec 1|Out-Null;$ready=$true;break}catch{Start-Sleep -Milliseconds 250}}
 if(-not$ready){throw 'Private test WebView did not start'}
 node tests/portable-startup-native.cjs
 if($LASTEXITCODE -ne 0){throw 'Native portable startup check failed'}
 python tests/portable-startup-profile.py verify
 if($LASTEXITCODE -ne 0){throw 'Private state preservation check failed'}
 Write-Output ('Synthetic evidence: '+$profileRoot)
} finally {
 if($process -and -not$process.HasExited -and $process.Path -eq $binary){Stop-Process -Id $process.Id}
 $env:MCOLLAB_TEST_DATA_DIR=$previousRoot;$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=$previousArgs;$env:MCOLLAB_PORTABLE_HELPER=$previousHelper
}
