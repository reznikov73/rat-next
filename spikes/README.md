# Спайки (P0)

Исследовательские мини-проекты. Каждый — **standalone**-пакет (пустой `[workspace]`,
исключён из основного workspace через `exclude`), у каждого — свой `REPORT.md` с
фактическими результатами.

| Спайк | Цель | Статус |
|---|---|---|
| [quic_echo](quic_echo/) | quinn + rustls(ring): собрать и запустить QUIC echo на localhost | ✅ built + ran (`QUIC_OK`) |
| [dxgi_capture](dxgi_capture/) | DXGI Desktop Duplication: захват 1920×1080, сохранение PNG | ✅ built + ran (76 кадров / 5 с, PNG в `out/`) |
| ui_egui / ui_iced | Сравнение UI-китов для вьюера | ⏳ P0-Б (не начат) |
| ice_str0m | ICE/STUN (NAT traversal) | ⏳ P0-Б (не начат) |
| encode (QSV/NVENC/AMF) | Аппаратный H.264-энкодер | ⏳ P0-Б (не начат) |

Запуск любого спайка (из корня репозитория):

```powershell
. .\scripts\devshell.ps1
cd spikes\<имя>
cargo run
```
