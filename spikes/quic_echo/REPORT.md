# REPORT — спайк quic_echo

**Дата:** 2026-10-01 · **Статус:** ✅ built + ran OK

## Цель

Проверить, что стек QUIC (quinn + rustls с провайдером `ring`) собирается и работает
на целевой машине (Windows 10, MSVC toolchain), до того как на него закладывается
транспортный слой (`crates/transport`).

## Как запустить

```powershell
. .\scripts\devshell.ps1
cd spikes\quic_echo
cargo run
```

## Результат (фактический вывод)

```
[server] listening on 127.0.0.1:50156
[client] connected: 127.0.0.1:50156
[client] ok: ping-0 -> pong-0
[client] ok: ping-1 -> pong-1
[client] ok: ping-2 -> pong-2
[client] ok: ping-3 -> pong-3
[client] ok: ping-4 -> pong-4
QUIC_OK: 5 echo messages verified
```

## Версии

- rustc/cargo 1.98.1 (stable-x86_64-pc-windows-msvc), MSVC 14.44.35207
- quinn 0.11.12 / quinn-proto 0.11.19 / rustls 0.23.45 / ring 0.17.14 / tokio 1.53.1 / rcgen 0.13.2
- Полная сборка дерева зависимостей ~5 мин (i5-7600, 4 ядра)

## Выводы

- Связка quinn + rustls (ring) подтверждена: собирается без cmake/nasm (провайдер aws-lc-rs не использовался).
- Паттерн «сервер + клиент + двунаправленные стримы» работает; переносим в `crates/transport` (фазы P2–P4).
- В спайке: self-signed сертификат + отключённая проверка (ТОЛЬКО для теста).
  В продукте — device-сертификаты (mTLS) и пиннинг Ed25519-идентичностей (см. §3.4 проектного документа).

## Следующие шаги

- P0-Б: ICE-спайк на str0m (+ оценка libdatachannel как fallback).
- P2: мультиплексирование каналов поверх QUIC (`crates/mux`).
