$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if(Test-Path '.tools/msvc-sysroot'){
 $env:CARGO_HOME="$PWD/.tools/cargo"
 $env:RUSTUP_HOME="$PWD/.tools/rustup"
 $env:RUSTUP_TOOLCHAIN='1.90.0-x86_64-pc-windows-msvc'
 $llvmRoot="$PWD/.tools/llvm-mingw-20261006-ucrt-x86_64/bin"
 $env:PATH="$llvmRoot;$PWD/.tools/cargo/bin;$env:PATH"
 $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER="$llvmRoot/lld-link.exe"
 $env:CC="$llvmRoot/clang-cl.exe"; $env:CXX=$env:CC
 $env:AR="$llvmRoot/llvm-lib.exe"; $env:RC="$llvmRoot/llvm-rc.exe"
 $vcRoot=Get-ChildItem '.tools/msvc-sysroot/VC/Tools/MSVC' -Directory | Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty FullName
 $sdkRoot="$PWD/.tools/msvc-sysroot/Windows Kits/10"
 $sdkVersion=Get-ChildItem "$sdkRoot/Include" -Directory | Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty Name
 $env:LIB="$vcRoot/lib/x64;$sdkRoot/Lib/$sdkVersion/um/x64;$sdkRoot/Lib/$sdkVersion/ucrt/x64"
 $env:INCLUDE="$vcRoot/include;$sdkRoot/Include/$sdkVersion/ucrt;$sdkRoot/Include/$sdkVersion/um;$sdkRoot/Include/$sdkVersion/shared;$sdkRoot/Include/$sdkVersion/winrt"
}

# Local builds only. Don't commit the downloaded registration JSON.
$googleOAuthPathFile=Join-Path $PWD '.tools/google-oauth-path.txt'
if(Test-Path -LiteralPath $googleOAuthPathFile){
 $googleOAuthFilePath=(Get-Content -LiteralPath $googleOAuthPathFile -Raw).Trim()
 $googleOAuthRegistration=Get-Content -LiteralPath $googleOAuthFilePath -Raw | ConvertFrom-Json
 if(-not $googleOAuthRegistration.installed.client_id){throw 'Google registration must be a Desktop OAuth client.'}
 if(-not $env:MCOLLAB_GOOGLE_CLIENT_ID){$env:MCOLLAB_GOOGLE_CLIENT_ID=$googleOAuthRegistration.installed.client_id}
 if($env:MCOLLAB_GOOGLE_CLIENT_ID -eq $googleOAuthRegistration.installed.client_id -and -not $env:MCOLLAB_GOOGLE_CLIENT_SECRET){
  $env:MCOLLAB_GOOGLE_CLIENT_SECRET=$googleOAuthRegistration.installed.client_secret
 }
 Remove-Variable googleOAuthRegistration
}
