# build_cs_probe.ps1 - build the cs-proc host / type source for cs_probe (ffi_value_exchange_plan.md 1-1).
# The csproj copies cs_probe.exe / cs_probe.dll / cs_probe.runtimeconfig.json to this directory,
# where import[cs-proc] cs_probe finds them. bin/ and obj/ are not tracked.
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Push-Location $here
try { dotnet build -c Debug -nologo -v q; if ($LASTEXITCODE -ne 0) { Write-Error "dotnet build failed ($LASTEXITCODE)" } }
finally { Pop-Location }
