# Сборка портативного комплекта Rat Next («папка переноса»).
# Использование: powershell -ExecutionPolicy Bypass -File scripts\build-portable.ps1
# Результат:     dist\rat-next-<ГГГГ-ММ-ДД>\ — папку целиком копируют на машины.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$date = Get-Date -Format 'yyyy-MM-dd'

# Занятые процессы из target\release блокируют замену exe при сборке.
$all = Get-Process rat, rat-agent, rat-viewer, rat-relay, rat-elevate -ErrorAction SilentlyContinue
$busy = $all | Where-Object { $_.Path -like "$root\target\*" }
if ($busy) {
    Write-Host 'ОШИБКА: запущены сборки из target\release — закрыть и повторить:' -ForegroundColor Red
    $busy | ForEach-Object { Write-Host ("  pid=" + $_.Id + "  " + $_.Path) -ForegroundColor Red }
    exit 1
}
if ($all) {
    Write-Host 'Примечание: другие копии Rat Next запущены (сборку не блокируют).' -ForegroundColor Yellow
}

Set-Location $root
. (Join-Path $PSScriptRoot 'devshell.ps1')

Write-Host '== сборка release (rat, agent, viewer, relay, elevate) ==' -ForegroundColor Cyan
cargo build --release -p rat -p rat-agent -p rat-viewer -p rat-relay -p rat-elevate --features rat/ffmpeg,rat-agent/ffmpeg,rat-viewer/ffmpeg
if ($LASTEXITCODE -ne 0) { throw "cargo build завершился с кодом $LASTEXITCODE" }

$out = Join-Path $root "dist\rat-next-$date"
New-Item -ItemType Directory -Force -Path $out | Out-Null
$rel = Join-Path $root 'target\release'
foreach ($f in 'rat.exe', 'rat-agent.exe', 'rat-viewer.exe', 'rat-relay.exe', 'rat-elevate.exe') {
    Copy-Item (Join-Path $rel $f) $out -Force
}
foreach ($d in 'avcodec-63.dll', 'avdevice-63.dll', 'avfilter-12.dll', 'avformat-63.dll', 'avutil-61.dll', 'swresample-7.dll', 'swscale-10.dll') {
    Copy-Item (Join-Path $rel $d) $out -Force
}
# Запускалки и инструкция — из шаблона. device.json/machines.json НЕ кладём:
# на новой машине приложение создаст их само (ID — из отпечатка машины).
Copy-Item (Join-Path $root 'scripts\portable\*') $out -Force

Write-Host ("Готово: " + $out) -ForegroundColor Green
Get-ChildItem $out | ForEach-Object { Write-Host ("  " + $_.Name) }