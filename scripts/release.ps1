# release.ps1 -- raise the version, rebuild, tag, and deploy arrow.exe / the VSIX / ar_config.json.
#
#   ./scripts/release.ps1 -Version 0.4.1 [-Tag v_0_4_1] [-DeployDir ..\Arrow] [-CommitTrailer '...']
#
# Steps (the `release` skill has the reasons and the traps):
#   1. bump     the version in Cargo.toml, crates/arrow-frontend/Cargo.toml, vscode-extension/package.json,
#               package-lock.json and make-vsix.ps1 (scripts/release_files.py bump; stops if any site moved)
#   2. build    cargo build --release (arrow.exe) and make-vsix.ps1 (arrow-<Version>.vsix, wasm included)
#   3. commit   the bump and the new VSIX (the old VSIX is removed from git), on the current branch; no push
#   4. tag      an annotated tag on that commit
#   5. deploy   move <DeployDir>\{arrow.exe, arrow-<prev>.vsix, ar_config.json} to <DeployDir>\old\<prev>\,
#               then copy the new exe / VSIX and a deploy config (scripts/release_files.py config: the repo's
#               ar_config.json, keeping the deployed machine-specific values, without the "test" section).
#               bridge\js_bridge.cjs is replaced too when it changed (the old one goes to old\<prev>\bridge\).
#   6. smoke    run the deployed exe once from <DeployDir>
#
# Re-running after a failure is safe: a step already done is skipped (version already raised and its VSIX
# committed / tag already on HEAD); the backup folder is never overwritten.
#
# ASCII-only on purpose: PowerShell 5.1 reads BOM-less .ps1 as ANSI (Japanese text lives in release_files.py).

param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$Tag,
    [string]$DeployDir,
    # Version of the files now in $DeployDir (taken from its arrow-<v>.vsix when omitted).
    [string]$PrevVersion,
    # Appended to the commit message (e.g. a Co-Authored-By line).
    [string]$CommitTrailer = '',
    [int]$TimeoutSec = 3600
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $Tag)       { $Tag = 'v_' + ($Version -replace '\.', '_') }
if (-not $DeployDir) { $DeployDir = Join-Path (Split-Path -Parent $repo) 'Arrow' }
$DeployDir = [System.IO.Path]::GetFullPath($DeployDir)
$helper = Join-Path $PSScriptRoot 'release_files.py'
$vsixRel = "vscode-extension/arrow-$Version.vsix"
$vsix = Join-Path $repo $vsixRel
$exe  = Join-Path $repo 'target\release\arrow.exe'

if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "-Version must look like 0.4.1 (got '$Version')" }

function Step([string]$text) { Write-Host "== $text" -ForegroundColor Cyan }

# Run a native command with the console inherited, killing its tree when it exceeds the limit.
function Invoke-Limited([string]$file, [string]$argLine, [string]$workDir = $repo) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $file
    $psi.Arguments = $argLine
    $psi.WorkingDirectory = $workDir
    $psi.UseShellExecute = $false
    $p = [System.Diagnostics.Process]::Start($psi)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        & taskkill /T /F /PID $p.Id 2>&1 | Out-Null
        Write-Host ("ABNORMAL TERMINATION: '{0} {1}' did not finish in {2} s (killed)" -f $file, $argLine, $TimeoutSec) -ForegroundColor Red
        exit 2
    }
    $p.WaitForExit()
    if ($p.ExitCode -ne 0) { throw "'$file $argLine' failed (exit $($p.ExitCode))" }
}

function Git([string]$argLine) { Invoke-Limited 'git' $argLine }

# Same, but capture stdout+stderr (returned with the exit code) instead of showing them.
function Invoke-Captured([string]$file, [string]$argLine, [string]$workDir = $repo) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $file
    $psi.Arguments = $argLine
    $psi.WorkingDirectory = $workDir
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $p = [System.Diagnostics.Process]::Start($psi)
    $oTask = $p.StandardOutput.ReadToEndAsync()
    $eTask = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        & taskkill /T /F /PID $p.Id 2>&1 | Out-Null
        Write-Host ("ABNORMAL TERMINATION: '{0} {1}' did not finish in {2} s (killed)" -f $file, $argLine, $TimeoutSec) -ForegroundColor Red
        exit 2
    }
    $p.WaitForExit()
    return @{ Out = $oTask.Result; Err = $eTask.Result; Code = $p.ExitCode }
}

# True when <path> is in the HEAD commit (ls-tree: no stderr, which PS 5.1 would turn into an error).
function In-Head([string]$path) { return [bool](& git ls-tree --name-only HEAD -- $path) }

function Read-CargoVersion {
    $t = [System.IO.File]::ReadAllText((Join-Path $repo 'Cargo.toml'), [System.Text.Encoding]::UTF8)
    $m = [regex]::Match($t, 'name = "arrow"\r?\nversion = "([^"]+)"')
    if (-not $m.Success) { throw 'Cargo.toml: the [package] version line of "arrow" was not found' }
    return $m.Groups[1].Value
}

# ---------------------------------------------------------------- preconditions
Step 'preconditions'
Push-Location $repo
try {
    $dirty = & git status --porcelain
    if ($dirty) { $dirty | ForEach-Object { Write-Host "  $_" }; throw 'the working tree is not clean (commit or stash first)' }

    if (-not (Test-Path $DeployDir)) { throw "deploy folder not found: $DeployDir" }
    if (-not $PrevVersion) {
        $old = @(Get-ChildItem $DeployDir -Filter 'arrow-*.vsix' -File)
        if ($old.Count -ne 1) { throw "cannot tell the deployed version: $($old.Count) arrow-*.vsix in $DeployDir (pass -PrevVersion)" }
        $PrevVersion = $old[0].BaseName.Substring('arrow-'.Length)
    }
    if ($PrevVersion -eq $Version) { throw "$DeployDir already holds arrow-$Version.vsix" }
    $backup = Join-Path $DeployDir "old\$PrevVersion"
    if ((Test-Path $backup) -and @(Get-ChildItem $backup -Force).Count -gt 0) {
        throw "backup folder already has files: $backup (never overwritten; move it away first)"
    }
    foreach ($f in @('arrow.exe', "arrow-$PrevVersion.vsix", 'ar_config.json')) {
        if (-not (Test-Path (Join-Path $DeployDir $f))) { throw "deployed file missing: $DeployDir\$f" }
    }

    & git rev-parse -q --verify "refs/tags/$Tag" | Out-Null
    $tagExists = ($LASTEXITCODE -eq 0)
    $head = (& git rev-parse HEAD).Trim()
    if ($tagExists) {
        $tagged = (& git rev-parse "$Tag^{commit}").Trim()
        if ($tagged -ne $head) { throw "tag $Tag already exists on $tagged (HEAD is $head)" }
    }

    $current = Read-CargoVersion
    Write-Host "  repo version $current -> $Version, deployed $PrevVersion, tag $Tag, deploy to $DeployDir"
    Write-Host "  branch $((& git branch --show-current).Trim())"

    # ------------------------------------------------------------ 1. bump
    $bumped = $false
    if ($current -ne $Version) {
        if ($tagExists) { throw "tag $Tag exists but the version is still $current" }
        Step "bump $current -> $Version"
        Invoke-Limited 'python' ('-X utf8 "{0}" bump "{1}" {2} {3}' -f $helper, $repo, $current, $Version)
        $bumped = $true
    } else {
        Step "version is already $Version (bump skipped)"
        if (-not (In-Head $vsixRel)) { throw "version is already $Version but $vsixRel is not committed; build and commit it, then re-run" }
    }

    # ------------------------------------------------------------ 2. build
    Step 'cargo build --release'
    Invoke-Limited 'cargo' 'build --release'
    if (-not (Test-Path $exe)) { throw "arrow.exe not produced: $exe" }

    if ($bumped) {
        Step 'make-vsix.ps1'
        Invoke-Limited 'powershell' ('-NoProfile -ExecutionPolicy Bypass -File "{0}"' -f (Join-Path $repo 'vscode-extension\make-vsix.ps1'))
        if (-not (Test-Path $vsix)) { throw "VSIX not produced: $vsix" }

        # ------------------------------------------------------------ 3. commit
        Step 'commit'
        $oldVsix = "vscode-extension/arrow-$current.vsix"
        if (In-Head $oldVsix) { Git "rm -q `"$oldVsix`"" }
        Git 'add -u'
        Git "add `"$vsixRel`""
        $msg = Join-Path ([System.IO.Path]::GetTempPath()) "arrow_release_msg_$Version.txt"
        $trailerArg = if ($CommitTrailer) { '"' + ($CommitTrailer -replace '"', '\"') + '"' } else { '' }
        Invoke-Limited 'python' ('-X utf8 "{0}" message "{1}" {2} {3} "{4}" {5}' -f $helper, $msg, $PrevVersion, $Version, $DeployDir, $trailerArg)
        Git "commit -q -F `"$msg`""
        Remove-Item $msg
        & git log --oneline -1
    }

    # ------------------------------------------------------------ 4. tag
    $head = (& git rev-parse HEAD).Trim()
    if (-not $tagExists) {
        Step "tag $Tag"
        Git "tag -a $Tag -m `"Arrow $Version`""
    } else {
        Step "tag $Tag already on HEAD (skipped)"
    }
} finally { Pop-Location }

# ---------------------------------------------------------------- 5. deploy
Step "back up $PrevVersion -> $backup"
New-Item -ItemType Directory -Force $backup | Out-Null
foreach ($f in @('arrow.exe', "arrow-$PrevVersion.vsix", 'ar_config.json')) {
    Move-Item (Join-Path $DeployDir $f) (Join-Path $backup $f)
    Write-Host "  moved $f"
}

Step "deploy $Version -> $DeployDir"
Copy-Item $exe  (Join-Path $DeployDir 'arrow.exe')
Copy-Item $vsix (Join-Path $DeployDir "arrow-$Version.vsix")
Invoke-Limited 'python' ('-X utf8 "{0}" config "{1}" "{2}" "{3}"' -f $helper, (Join-Path $repo 'ar_config.json'), (Join-Path $backup 'ar_config.json'), (Join-Path $DeployDir 'ar_config.json'))
foreach ($pair in @(@($exe, 'arrow.exe'), @($vsix, "arrow-$Version.vsix"))) {
    $a = (Get-FileHash $pair[0]).Hash; $b = (Get-FileHash (Join-Path $DeployDir $pair[1])).Hash
    if ($a -ne $b) { throw "copy mismatch: $($pair[1])" }
    Write-Host "  copied $($pair[1])"
}

$bridgeSrc = Join-Path $repo 'bridge\js_bridge.cjs'
$bridgeDst = Join-Path $DeployDir 'bridge\js_bridge.cjs'
if ((Test-Path $bridgeDst) -and ((Get-FileHash $bridgeSrc).Hash -ne (Get-FileHash $bridgeDst).Hash)) {
    New-Item -ItemType Directory -Force (Join-Path $backup 'bridge') | Out-Null
    Move-Item $bridgeDst (Join-Path $backup 'bridge\js_bridge.cjs')
    Copy-Item $bridgeSrc $bridgeDst
    Write-Host '  bridge\js_bridge.cjs changed: replaced (old one backed up)'
} else {
    Write-Host '  bridge\js_bridge.cjs unchanged'
}

# ---------------------------------------------------------------- 6. smoke
Step 'smoke run of the deployed exe'
$smoke = Join-Path $DeployDir '__release_smoke.ar'
[System.IO.File]::WriteAllText($smoke, "let x: int = 1 + 2`nprint(x)`n", (New-Object System.Text.UTF8Encoding($false)))
try {
    $r = Invoke-Captured (Join-Path $DeployDir 'arrow.exe') ('"{0}"' -f $smoke) $DeployDir
    if ($r.Code -ne 0 -or $r.Out.Trim() -ne '3') { Write-Host $r.Out; Write-Host $r.Err -ForegroundColor Red; throw "smoke run failed (exit $($r.Code))" }
    Write-Host '  ok (printed 3)'
} finally { Remove-Item $smoke -ErrorAction SilentlyContinue }

Write-Host ''
Write-Host "released $Version (tag $Tag) to $DeployDir; previous files in $backup" -ForegroundColor Green
Write-Host 'push is NOT done (git push / git push origin $Tag when wanted)'
