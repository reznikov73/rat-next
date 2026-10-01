# REPORT — спайк video_loop (P1: локальный видео-луп)

**Дата:** 2026-10-01 · **Статус:** ✅ работает: 1080p, ~29 fps, без сети

## Цель

Проверить сквозной локальный конвейер «захват → энкод → декод → показ» и измерить
бюджет стадий (фаза P1 проектного плана).

## Архитектура спайка

- **Поток A:** пейсинг 30 fps → DXGI Desktop Duplication → staging readback →
  конверсия BGRA→NV12 (swscale) → отправка кадра в энкодер (libavcodec: `h264_qsv`,
  fallback `libx264`).
- **Поток B:** приём пакетов → декод H.264 (libavcodec, software) → конверсия YUV→RGBA
  (swscale) → кадр в UI.
- **UI:** egui/eframe (wgpu, DX12), текстура 1080p + строка статуса с метриками.
- **C-шим** (`src/shim.c`) инкапсулирует libavcodec/swscale; Rust-сторона — тонкие биндинги.
- FFmpeg: shared-сборка (BtbN win64-gpl-shared) в `E:\dev\Tools\ffmpeg-shared`;
  DLL копируются к exe автоматически (build.rs).

## Как запустить

```powershell
. .\scripts\devshell.ps1
cd spikes\video_loop
cargo run --release                # интерактивно (закрыть окно для остановки)
cargo run --release -- --smoke=20  # 20 c, затем авто-сводка в консоль
```

Кодек можно переопределить: `$env:VIDEO_CODEC="libx264"`.

## Результаты (release, 1080p, smoke 20 c)

```
codec=h264_qsv frames=573 dropped=0 errors=0 fps=29.2
stage            avg      p50      p95
capture+conv   16.92ms  16.52ms  22.44ms
encode         29.24ms  39.22ms  45.38ms   ("send -> пакет", асинхронная, перекрывается)
decode+conv    11.76ms  10.96ms  17.62ms
e2e (internal) 57.92ms  66.95ms  77.65ms
present gap    avg 12.54ms  p95 32.12ms
e2e wall (capture->present): avg 70.43ms  p95 101.80ms
```

## Выводы

- Сквозной луп работает end-to-end на целевом железе; QSV-энкод подтверждён в лупе.
  За прогон: 0 дропов, 0 ошибок.
- Узкое место — CPU-путь: конверсии (swscale) и software-декод. Дальнейшие направления:
  GPU-конверсия (шейдер/видео-процессор), аппаратный декод (QSV/D3D11VA), меньше работы
  с полным кадром BGRA; при переносе в продукт — решение по интеграционному пути кодеков
  (Media Foundation / oneVPL / FFmpeg-библиотека) — фаза P2+.
- Метрика «encode» = «send → пакет готов»; из-за конвейеризации и асинхронности QSV она
  в значительной мере перекрывается другими стадиями; для суммарной задержки смотреть
  «e2e wall».
- Целевой ориентир ≤50 мс (LAN, P95) на этом CPU-пайплайне пока не достигнут
  (P95 ~102 мс); путь к цели — оптимизации выше + HW-декод (следующие фазы).
- Лицензионное примечание: в спайке — GPL-shared сборка FFmpeg (локальный эксперимент,
  не распространяется). Для продукта — отдельный анализ (динамический LGPL и/или vendor SDK).

## Артефакты

- `src/shim.c`, `src/ffi.rs`, `src/pipeline.rs`, `src/main.rs`, `build.rs`.
- Для визуальной проверки: `cargo run --release` (окно «rat video-loop P1»).
