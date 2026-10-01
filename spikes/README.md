# Спайки (P0)

Исследовательские мини-проекты. Каждый — **standalone**-пакет (свой `[workspace]`,
исключён из основного workspace через `exclude`), у каждого — свой `REPORT.md`
с фактическими результатами.

| Спайк | Цель | Статус |
|---|---|---|
| [quic_echo](quic_echo/) | quinn + rustls(ring): QUIC echo на localhost | ✅ ran (`QUIC_OK`) |
| [dxgi_capture](dxgi_capture/) | DXGI Desktop Duplication: захват 1920×1080, PNG | ✅ ran (55–76 кадров / 5 c) |
| [ui_egui](ui_egui/) | egui/eframe 0.36: макет вьюера + smoke | ✅ ran (нужен DX12-бэкенд; Vulkan Intel падает) |
| [ui_iced](ui_iced/) | iced 0.14: макет вьюера + smoke | ✅ ran |
| [ice_str0m](ice_str0m/) | ICE (крейт `is` из str0m 0.24): соединение двух агентов | ✅ ran (connected за ~12 мс) |
| [encode_qsv](encode_qsv/) | Аппаратный H.264 (Intel QSV) через FFmpeg | ✅ ran (~128 fps при 1080p) |

**Решение по UI-киту:** [UI_KIT_DECISION.md](UI_KIT_DECISION.md) — рекомендован **egui**, iced — резерв.

Запуск любого спайка (из корня репозитория):

```powershell
. .\scripts\devshell.ps1
cd spikes\<имя>
cargo run            # для UI-спайков: cargo run -- --smoke
```
