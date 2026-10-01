# Запуск VS Code с окружением Rat Next (cargo + MSVC).
# Предпочтительный способ открыть проект: тогда rust-analyzer, интегрированный
# терминал и отладка сразу получают всё необходимое.
#
# Использование:
#   powershell -ExecutionPolicy Bypass -File scripts\code.ps1
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'devshell.ps1')
Set-Location $root
code .
