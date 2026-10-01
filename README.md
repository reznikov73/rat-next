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

Фаза P0 завершена: каркас репозитория, схема v0, спайки `quic_echo`, `dxgi_capture`, `ui_egui`,
`ui_iced`, `ice_str0m`, `encode_qsv` — см. `spikes/README.md`; решение по UI-киту —
`spikes/UI_KIT_DECISION.md`.
