# Run all non-error .ar example files and report pass/fail
$ErrorActionPreference = "Continue"
$pass = 0
$fail = 0
$errors = @()

# These require interactive input, external DLLs, or are skipped intentionally
$skip = @(
    "debug_demo",
    "async_bench",
    "async_demo",
    "spider_render",
    "spider_solitaire",
    "rs_struct",
    "flat_bench",
    "flat_bench_interp",
    "flat_bench_module"
)

$examples = Get-ChildItem "examples\*.ar" | Where-Object {
    $name = $_.BaseName
    -not ($name -match "_error" -or $name -match "__errors" -or $skip -contains $name)
} | Sort-Object Name

foreach ($f in $examples) {
    $out = & ".\target\release\arrow.exe" $f.FullName 2>&1
    $exitCode = $LASTEXITCODE
    if ($exitCode -eq 0) {
        Write-Host "[PASS] $($f.Name)" -ForegroundColor Green
        $pass++
    } else {
        Write-Host "[FAIL] $($f.Name)" -ForegroundColor Red
        $errors += [PSCustomObject]@{ File = $f.Name; Output = ($out | Select-Object -Last 5) -join "`n" }
        $fail++
    }
}

Write-Host ""
Write-Host "Results: $pass passed, $fail failed"
if ($errors.Count -gt 0) {
    Write-Host ""
    Write-Host "=== Failures ==="
    foreach ($e in $errors) {
        Write-Host "--- $($e.File) ---" -ForegroundColor Yellow
        Write-Host $e.Output
    }
}
