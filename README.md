# Rat Next

Production-grade система удалённого администрирования Windows: агент-служба, вьюер оператора, relay/signaling. Переписывание legacy-решения на Rust. Утверждённый проектный документ — [`docs/ARCHITECTURE_DESIGN_v1.1.md`](docs/ARCHITECTURE_DESIGN_v1.1.md) (читать в первую очередь).

## Структура репозитория

- `crates/` — общие крейты ядра
  - `proto` — схема управляющего протокола (protobuf v0)
  - `common` — утилиты, конфиг, ошибки
  - `transport` — QUIC/ICE/relay-транспорт
  - `crypto` — E2E-крипто (Noise, идентичности, PAKE)
  - `mux` — мультиплексирование каналов
- `bins/` — компоненты: `agent`, `agent-helper`, `relay`, `signaling`, `viewer`
- `spikes/` — исследовательские спайки (вне cargo workspace)
- `scripts/` — devshell и проверки
- `docs/` — проектные документы

## Сборка и проверка (Windows)

Окружение разработки на подготовленной машине: Rust — `E:\dev\Rust`, MSVC BuildTools — `E:\dev\VSBuildTools`.
Свежие шеллы не получают переменные автоматически — сначала активируй окружение:

```powershell
. .\scripts\devshell.ps1      # dot-source из корня репозитория
cargo build --workspace
```

Полный локальный прогон (fmt + clippy + build + test, как в CI):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\check.ps1
```

CI: `.github/workflows/ci.yml` (заработает после публикации репозитория на GitHub).

## Статус

- **P0 завершена**: каркас, схема v0, спайки (`quic_echo`, `dxgi_capture`, `ui_egui`, `ui_iced`,
  `ice_str0m`, `encode_qsv`) — см. `spikes/README.md`; UI-кит: egui (`spikes/UI_KIT_DECISION.md`).
- **P1 работает**: локальный видео-луп «экран → QSV H.264 → декод → показ» — 1080p, ~29 fps,
  e2e ~70 мс; см. `spikes/video_loop/REPORT.md`.


## Установщик (setup.exe)

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-installer.ps1
```

Собирает `dist\rat-next-setup-<дата>.exe` (нужен Inno Setup 6: `winget install JRSoftware.InnoSetup`).

- Ставится без прав администратора — в профиль пользователя (`%LocalAppData%\Programs\Rat Next`);
  при запуске от администратора мастер предложит установку для всех пользователей.
- В мастере: ярлык в меню «Пуск» (+ значок на рабочий стол по желанию), опция
  «Запускать при входе в Windows» (создаёт задачу Планировщика), «Запустить после установки».
- Удаление — через «Установленные приложения»: снимает задачу автозапуска и удаляет файлы.

> **Права администратора.** Чтобы управлять окнами приложений, запущенных
> от имени администратора, сам агент должен работать с повышенными правами —
> иначе Windows (UIPI) не пропускает туда ввод: мышь/клавиатура действуют
> только в обычных (непривилегированных) окнах. Задача автозапуска ставится
> с повышенными правами; при ручном запуске используйте
> «Запуск от имени администратора».