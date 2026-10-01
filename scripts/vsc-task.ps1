# Универсальный runner для задач VS Code:
# активирует окружение Rat Next (cargo + MSVC) и выполняет cargo-команду в нужном каталоге.
#
# Пример вызова:
#   powershell -File scripts\vsc-task.ps1 -Dir spikes/ui_egui -Cmd "run -- --smoke"
param(
    [string]$Dir = '.',
    [string]$Cmd = 'build'
)

$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'devshell.ps1')

Set-Location (Join-Path $root $Dir)

# ВАЖНО: @( ... ) — чтобы одиночный элемент остался массивом
# (сплат скалярной строки разворачивает её по символам!).
$argv = @($Cmd.Split(' ') | Where-Object { $_ -ne '' })
& cargo @argv
exit $LASTEXITCODE
