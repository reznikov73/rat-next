# Локальный аналог CI: fmt + clippy + build + test по workspace.
#
# Запуск:  powershell -NoProfile -ExecutionPolicy Bypass -File scripts\check.ps1
$ErrorActionPreference = 'Continue'
Set-Location (Split-Path -Parent $PSScriptRoot)

. (Join-Path $PSScriptRoot 'devshell.ps1')

$checks = @(
    @('cargo', 'fmt', '--all', '--check'),
    @('cargo', 'clippy', '--workspace', '--all-targets', '--', '-D', 'warnings'),
    @('cargo', 'build', '--workspace'),
    @('cargo', 'test', '--workspace')
)

$failed = $false
foreach ($c in $checks) {
    $cmdline = $c -join ' '
    Write-Host "==> $cmdline" -ForegroundColor Cyan
    & $c[0] @($c[1..($c.Count - 1)])
    if ($LASTEXITCODE -ne 0) {
        $failed = $true
        Write-Host "FAILED: $cmdline" -ForegroundColor Red
        break
    }
}

if ($failed) { exit 1 }
Write-Host 'ALL CHECKS PASSED' -ForegroundColor Green
