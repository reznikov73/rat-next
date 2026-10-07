# Сборка установщика Rat Next (Inno Setup 6).
# Использование:  powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
# Результат:      dist\rat-next-setup-<ГГГГ-ММ-ДД>.exe
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$date = Get-Date -Format 'yyyy-MM-dd'

Write-Host '== 1/3: сборка релиза ==' -ForegroundColor Cyan
Push-Location $root
try {
    . .\scripts\devshell.ps1
    cargo build --release -p rat -p rat-relay -p rat-elevate --features rat/ffmpeg
    if ($LASTEXITCODE -ne 0) { throw "cargo build завершился с кодом $LASTEXITCODE (возможно, запущенный rat.exe блокирует сборку)" }
} finally {
    Pop-Location
}

Write-Host '== 2/3: подготовка payload ==' -ForegroundColor Cyan
$payload = Join-Path $root 'dist\payload'
New-Item -ItemType Directory -Force -Path $payload | Out-Null
$rel = Join-Path $root 'target\release'
Copy-Item (Join-Path $rel 'rat.exe') $payload -Force
Copy-Item (Join-Path $rel 'rat-elevate.exe') $payload -Force
Copy-Item (Join-Path $rel 'rat-relay.exe') $payload -Force
foreach ($d in 'avcodec-63.dll','avdevice-63.dll','avfilter-12.dll','avformat-63.dll','avutil-61.dll','swresample-7.dll','swscale-10.dll') {
    Copy-Item (Join-Path $rel $d) $payload -Force
}
$readme = Get-ChildItem (Join-Path $root 'dist') -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like 'rat-next-*' } | Sort-Object Name -Descending |
    ForEach-Object { Join-Path $_.FullName 'README.txt' } | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $readme) {
    $tpl = Join-Path $root 'scripts\portable\README.txt'
    if (Test-Path $tpl) { $readme = $tpl }
}
if ($readme) { Copy-Item $readme $payload -Force } else { Write-Warning 'README.txt не найден в dist' }
$ico = Get-ChildItem (Join-Path $rel 'build') -Recurse -Filter 'rat.ico' -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($ico) { Copy-Item $ico.FullName (Join-Path $payload 'rat-next.ico') -Force } else { Write-Warning 'rat.ico не найден' }

Write-Host '== 3/3: сборка установщика ==' -ForegroundColor Cyan
$iscc = Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'
if (-not (Test-Path $iscc)) { $iscc = 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe' }
if (-not (Test-Path $iscc)) { throw 'ISCC.exe не найден — установи Inno Setup 6: winget install JRSoftware.InnoSetup' }
& $iscc "/DDateStamp=$date" "/DPayloadDir=$payload" (Join-Path $root 'scripts\installer.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC завершился с кодом $LASTEXITCODE" }
$out = Join-Path $root "dist\rat-next-setup-$date.exe"
Get-Item $out | ForEach-Object { Write-Host ("Готово: " + $_.FullName + " (" + [math]::Round($_.Length / 1MB, 1) + ' МБ)') -ForegroundColor Green }