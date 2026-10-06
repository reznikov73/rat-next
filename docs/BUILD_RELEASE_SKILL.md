---
name: rat-build-release
description: >-
  Порядок сборки и выпуска Rat Next (E:\Work\GIGA\github\Rat): окружение devshell,
  release-сборка rat/relay/elevate, установщик Inno Setup, портативный комплект
  («папка переноса») dist\rat-next-<дата>, проверки и типовые грабли (занятые exe,
  BOM у .ps1). Использовать при любой сборке exe, установщика или комплекта для машин.
---

# Навык: сборка и выпуск Rat Next

## Рабочий каталог и окружение
- Репозиторий: `E:\Work\GIGA\github\Rat`; все команды — из его корня.
- Перед любым cargo: `. .\scripts\devshell.ps1` (активирует cargo + MSVC x64).
  Без него сборка не найдёт линкер.
- PowerShell 5.1: файлы `.ps1` с кириллицей сохранять **только UTF-8 с BOM**
  (без BOM парсер ломается на кириллице).

## Проблема №1 — занятые exe (самая частая)
Запущенные копии из `target\release` (rat.exe / rat-relay.exe / …) держат
файлы, и сборка падает («failed to remove», «Access denied», os error 32).
Перед сборкой:
- `Get-Process rat,rat-agent,rat-viewer,rat-relay,rat-elevate -ErrorAction SilentlyContinue`
- остановить службу, если крутится: `sc.exe stop RatNext` (и `RatNextRelay`), подождать.
- свежий бинарь, если основной файл занят, лежит в `target\release\deps\rat.exe`.

## Порядок выпуска
1) Проверки (перед выпуском — обязательно):
   `powershell -NoProfile -ExecutionPolicy Bypass -File scripts\check.ps1`
   → «ALL CHECKS PASSED» и код 0 (fmt --check, clippy, build, test).
2) Установщик: `... -File scripts\build-installer.ps1`
   — собирает rat, rat-relay, rat-elevate (release, ffmpeg), готовит `dist\payload`
   (exe + 7 DLL + README.txt + иконка) и вызывает ISCC (Inno Setup 6.
   Путь: `%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe`).
   Результат: `dist\rat-next-setup-<ГГГГ-ММ-ДД>.exe`.
3) Портативный комплект («папка переноса»): `... -File scripts\build-portable.ps1`
   — собирает rat, rat-agent, rat-viewer, rat-relay, rat-elevate и наполняет
   `dist\rat-next-<ГГГГ-ММ-ДД>\`: все exe, 7 DLL, четыре «Запустить ….cmd»,
   `README.txt` (шаблоны — `scripts\portable\`). Папку целиком копируют на машины.
   `device.json`/`machines.json` НЕ кладём — приложение создаёт их само
   (ID — из отпечатка машины).
4) Коммит и пуш — ОБЯЗАТЕЛЬНО в конце любой работы (даже если только собрал):
   - `git add` изменённых исходников/скриптов/шаблонов (каталог `dist` не добавляем —
     он в .gitignore);
   - коммит с русским сообщением (что сделано и зачем);
   - `$env:GIT_TERMINAL_PROMPT='0'; git push origin main`
     (если push упал из-за сети — просто повторить);
   - проверить: `git status` чистый, `git log` — локальная ветка = origin/main.

## Где смотреть результат
- Бинарники: `target\release\rat.exe`, `rat-relay.exe`, `rat-elevate.exe`,
  `rat-agent.exe`, `rat-viewer.exe`.
- Установщик: `dist\rat-next-setup-<дата>.exe`.
- Комплект: `dist\rat-next-<дата>\`.
- Вывод сборок — в терминале VS Code (панель «Терминал», вкладка задачи):
  успех виден по «Finished … release» и строке «Готово: <путь>».

## VS Code задачи — что и в каком порядке
- `check: fmt + clippy + build + test` — проверки (по умолчанию: Ctrl+Shift+B).
- `build: installer` — полный выпуск установщика (шаг 2).
- `build: portable (папка переноса)` — комплект для переноса (шаг 3).
- `build: rat app (release)` / `build: rat-relay (release)` — точечно.
- Задачи `spikes/*` — исторические, для продукта не нужны.
Запуск: `Ctrl+Shift+P → Tasks: Run Task` → нужная задача; вывод — в терминале.

## Правила
- `dist/` игнорируется гитом («папка переноса»): в коммит идут только исходники,
  `scripts\`, шаблоны `scripts\portable\`, `.vscode\tasks.json`.
- Не коммитить `device.json`, `machines.json`, логи, каталоги агентных инструментов.
- **Работа не считается законченной без коммита и пуша в `origin/main`** —
  никогда не оставлять изменения только локально.
- Сообщения коммитов — по-русски.