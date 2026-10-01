# REPORT — спайк ice_str0m

**Дата:** 2026-10-01 · **Статус:** ✅ built + ran OK (соединение за ~12 мс)

## Цель

Проверить ICE-стек (NAT traversal) из str0m 0.24 на нашей платформе до того, как он
станет основой транспортного слоя (`crates/transport`).

## Состав

- Крейт `is` 0.11.1 — это ICE-слой из str0m 0.24 (`str0m::ice` ре-экспортирует его же).
- Сценарий: два IceAgent (controlling / controlled), host-кандидаты на 127.0.0.1:41000/41001;
  креды и кандидаты передаются напрямую (эмуляция signaling-сервера), Transmit'ы
  доставляются in-memory (эмуляция UDP-сокетов).

## Результат (фактический вывод)

```
ICE spike start (crate `is`)
candidates exchanged: a=127.0.0.1:41000 b=127.0.0.1:41001
[a] event: IceRestart(...)
[a] event: IceConnectionStateChange(Checking)
[a] event: DiscoveredRecv { proto: Udp, source: 127.0.0.1:41001 }
[a] event: NominatedSend { proto: Udp, source: 127.0.0.1:41000, destination: 127.0.0.1:41001 }
[a] event: IceConnectionStateChange(Completed)
[b] ... (аналогично)
ICE_OK: both connected in 12.3707ms (7 transmits); states: a=Completed b=Completed
```

## Версии

- is 0.11.1 (str0m 0.24), str0m-proto 0.7.0; rustc 1.98.1 / MSVC 14.44.35207.

## Выводы / интеграционные заметки

- API пригоден для sans-IO интеграции: `poll_transmit()` / `handle_packet()` / `handle_timeout()` /
  `poll_event()` / `state()`; кандидаты — `Candidate::host(addr, Protocol::Udp)`;
  креды — `IceCreds::new()`.
- Крейт `is` собирается без crypto-бэкендов (ICE = STUN + HMAC-SHA1; зависимость `sha1`).
- **ВАЖНО:** полный `str0m` по умолчанию тянет `aws-lc-rs` (нужны cmake/nasm — на машине нет).
  Для продукта выбрать backend через feature-флаги (`wincrypto` / `openssl` / `rust-crypto`) —
  проверить при сборке транспорта (P2).
- Следующий шаг (P2–P4): реальные UDP-сокеты + STUN-сервер (srflx-кандидаты) + signaling-обмен,
  затем relay-fallback.
