# generate-codebase-map.ps1
# Regenerates the AUTO-TREE section of .claude/skills/codebase-map/SKILL.md.
# Run from anywhere; paths resolve relative to this script's location (repo root).
# Run this after creating / moving / renaming / deleting source files.
# Hand-written "Module Roles" text in the skill is left untouched — only the
# block between the AUTO-TREE markers is replaced.

$ErrorActionPreference = 'Stop'
$repo = $PSScriptRoot
$excludeDirs = @('__pycache__', 'node_modules', 'out', 'out_debug', 'target', '.git', 'bin', 'obj', 'math_output')

$script:fileCount = 0
$script:lineTotal = 0

function Get-LineCount([string]$path) {
    try { return [System.IO.File]::ReadAllLines($path).Count } catch { return 0 }
}

# Emits an indented file tree for $dir, files first then subdirectories.
# Only files whose extension is in $exts are listed; empty subtrees are omitted.
function Get-Tree([string]$dir, [string]$indent, [string[]]$exts) {
    $lines = @()
    $items = Get-ChildItem -LiteralPath $dir | Sort-Object PSIsContainer, Name
    foreach ($it in $items) {
        if ($it.PSIsContainer) {
            if ($excludeDirs -contains $it.Name) { continue }
            $sub = @(Get-Tree $it.FullName ($indent + '  ') $exts)
            if ($sub.Count -gt 0) {
                $lines += ($indent + $it.Name + '/')
                $lines += $sub
            }
        }
        elseif ($exts -contains $it.Extension) {
            $lc = Get-LineCount $it.FullName
            $script:fileCount += 1
            $script:lineTotal += $lc
            $lines += ('{0}{1} ({2})' -f $indent, $it.Name, $lc)
        }
    }
    return $lines
}

$tree = @()

# --- src/ (Rust) ---
$script:fileCount = 0; $script:lineTotal = 0
$body = @(Get-Tree (Join-Path $repo 'src') '  ' @('.rs', '.ars'))
$tree += ('src/  ({0} files, {1} lines)' -f $script:fileCount, $script:lineTotal)
$tree += $body

# --- impl_python/ ---
$script:fileCount = 0; $script:lineTotal = 0
$body = @(Get-Tree (Join-Path $repo 'impl_python') '  ' @('.py'))
$tree += ''
$tree += ('impl_python/  ({0} files, {1} lines)' -f $script:fileCount, $script:lineTotal)
$tree += $body

# --- vscode-extension/ (src/ + syntaxes/ only) ---
$script:fileCount = 0; $script:lineTotal = 0
$b1 = @(Get-Tree (Join-Path $repo 'vscode-extension\src') '    ' @('.ts'))
$b2 = @(Get-Tree (Join-Path $repo 'vscode-extension\syntaxes') '    ' @('.json'))
$tree += ''
$tree += ('vscode-extension/  ({0} files, {1} lines; src/ + syntaxes/ only)' -f $script:fileCount, $script:lineTotal)
$tree += '  src/'
$tree += $b1
$tree += '  syntaxes/'
$tree += $b2

# --- examples/ (directory level: recursive .ar counts) ---
$tree += ''
$tree += 'examples/  (recursive .ar counts per category)'
foreach ($d in (Get-ChildItem (Join-Path $repo 'examples') -Directory | Sort-Object Name)) {
    $n = @(Get-ChildItem $d.FullName -Recurse -File -Filter '*.ar').Count
    $tree += ('  {0}/ ({1} .ar)' -f $d.Name, $n)
}
$loose = @(Get-ChildItem (Join-Path $repo 'examples') -File -Filter '*.ar').Count
if ($loose -gt 0) { $tree += ('  ({0} loose .ar at top level)' -f $loose) }

# --- repo root files ---
$tree += ''
$tree += '(repo root)'
foreach ($f in (Get-ChildItem $repo -File | Where-Object { @('.md', '.ps1', '.json') -contains $_.Extension } | Sort-Object Name)) {
    $tree += ('  {0} ({1})' -f $f.Name, (Get-LineCount $f.FullName))
}

# --- splice into the skill file between markers ---
$skillPath = Join-Path $repo '.claude\skills\codebase-map\SKILL.md'
$content = [System.IO.File]::ReadAllText($skillPath)
$begin = '<!-- BEGIN AUTO-TREE -->'
$end = '<!-- END AUTO-TREE -->'
$i = $content.IndexOf($begin)
$j = $content.IndexOf($end)
if ($i -lt 0 -or $j -lt 0) { throw "AUTO-TREE markers not found in $skillPath" }

$block = "`n" + '```text' + "`n" + ($tree -join "`n") + "`n" + '```' + "`n" +
    ('_Generated {0} by generate-codebase-map.ps1_' -f (Get-Date -Format 'yyyy-MM-dd')) + "`n"
$newContent = $content.Substring(0, $i + $begin.Length) + $block + $content.Substring($j)
[System.IO.File]::WriteAllText($skillPath, $newContent)

Write-Host ('codebase-map tree regenerated: {0} lines written to {1}' -f $tree.Count, $skillPath)
