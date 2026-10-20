param([string]$BinaryPath='.tools/review2-target/debug/miliastra-collab.exe',[string]$FixtureRoot)
$ErrorActionPreference='Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
. ./scripts/environment.ps1
if(-not$FixtureRoot){throw 'Pass a disposable fixture folder with its throwaway signing key. Never use the release key.'}
$root=(Resolve-Path -LiteralPath $FixtureRoot).Path
if((Split-Path $root -Parent) -ne (Join-Path $PWD '.tools') -or (Split-Path $root -Leaf) -notlike 'portable-fixture-*'){throw 'Fixture folder must stay under .tools/portable-fixture-*'}
$env:MCOLLAB_PORTABLE_FIXTURE_ROOT=$root
$env:MCOLLAB_PORTABLE_HELPER=(Resolve-Path -LiteralPath $BinaryPath).Path
python tests/portable-helper.py
if($LASTEXITCODE -ne 0){throw 'Native helper checks failed'}
