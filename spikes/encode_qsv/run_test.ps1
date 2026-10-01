# Спайк P0-Б: проверка аппаратного H.264-энкодера (Intel QSV) через статический FFmpeg.
#
# Запуск:  powershell -NoProfile -ExecutionPolicy Bypass -File spikes\encode_qsv\run_test.ps1
$ErrorActionPreference = 'Continue'
$ff = 'E:\dev\Tools\ffmpeg\ffmpeg-9.0.2-essentials_build\bin\ffmpeg.exe'
if (-not (Test-Path $ff)) { Write-Host "ffmpeg не найден: $ff" -ForegroundColor Red; exit 1 }
Set-Location $PSScriptRoot
New-Item -ItemType Directory -Force 'out' | Out-Null

Write-Host '== 1) Энкодеры в сборке (qsv/nvenc/amf) ==' -ForegroundColor Cyan
& $ff -hide_banner -encoders 2>&1 | Select-String -Pattern 'qsv|nvenc|amf'

Write-Host '== 2) QSV encode: 1920x1080@30, 5 c (h264_qsv) ==' -ForegroundColor Cyan
$sw = [System.Diagnostics.Stopwatch]::StartNew()
& $ff -hide_banner -f lavfi -i testsrc2=size=1920x1080:rate=30 -t 5 -c:v h264_qsv -preset veryfast -global_quality 25 -y 'out\test_qsv.mp4' 2>&1 | Select-Object -Last 6
$sw.Stop()
Write-Host ("encode wall time: {0:N1} c" -f $sw.Elapsed.TotalSeconds)

Write-Host '== 3) Проверка файла (декод) ==' -ForegroundColor Cyan
& $ff -v error -i 'out\test_qsv.mp4' -f null - 2>&1
Write-Host "decode exit: $LASTEXITCODE"
Get-Item 'out\test_qsv.mp4' -EA SilentlyContinue | Select-Object Name, @{n='KB';e={[math]::Round($_.Length/1KB,0)}}
