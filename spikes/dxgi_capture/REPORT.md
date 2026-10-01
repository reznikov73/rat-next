# REPORT — спайк dxgi_capture

**Дата:** 2026-10-01 · **Статус:** ✅ built + ran OK (реальный захват выполнен)

## Цель

Проверить DXGI Desktop Duplication на целевой машине: создание D3D11-устройства
(hardware), получение кадров, копирование в staging-текстуру, чтение пикселей,
сохранение PNG.

## Как запустить

```powershell
. .\scripts\devshell.ps1
cd spikes\dxgi_capture
cargo run
```

Результаты — в `out/`.

## Результат (фактический вывод, финальный прогон)

```
[dxgi] D3D11 device created (hardware)
[dxgi] duplication OK: 1920x1080
[dxgi] saved out/frame_0.png
[dxgi] saved out/frame_1.png
[dxgi] saved out/frame_2.png
[dxgi] frames=55, timeouts=0 за 5.0s, сохранено 3 PNG -> out/
DXGI_OK
```

## Окружение

- Windows 10 22H2 (19045), консольная сессия 1 (активная)
- Intel HD Graphics 630 (i5-7600), драйвер 31.0.101.2112
- windows 0.58 / image 0.25.10, MSVC 14.44.35207

## Наблюдения

- Доставленные кадры: ~55–76 за 5 c (~11–15 fps при активном рабочем столе);
  при статичном экране `AcquireNextFrame` отдаёт таймауты — нормальное поведение DD
  (счётчик `timeouts` в выводе).
- **Первый кадр после старта duplication может быть пустым (наблюдали чёрный кадр).**
  В спайк добавлено `SKIP_FRAMES = 3` — сохраняем кадры после «прогрева».
  Для боевого пайплайна это же правило стоит учесть.
- PNG: 1920×1080, ~503 КБ каждый; содержимое — светлый UI (проверено локальным
  превью, без внешних сервисов). `out/*.png` — для визуального контроля
  (в git не коммитятся, см. `.gitignore`).
- `ACCESS_LOST` за время прогона не встретился; обработка предусмотрена (сообщение + break).

## Выводы

- Основной путь захвата подтверждён на целевой машине (hardware D3D11 + DD + CPU-чтение).
- Переносим паттерн в `bins/agent-helper` (фаза P1): пересоздание duplication при
  `ACCESS_LOST`/`DISPLAY_CHANGED`, перечисление всех output'ов (мультимонитор),
  dirty/move-rects из `DXGI_OUTDUPL_FRAME_INFO`.

## Следующие шаги

- P1: конвейер «захват → энкод» (на этой машине — Intel QSV; NVENC/AMF — на машинах
  с соответствующими GPU); измерение latency по стадиям.
- P0-Б: UI-спайк (egui/iced), ICE (str0m), проверка QSV.
