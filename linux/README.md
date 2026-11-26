# Rat Next для Linux

Здесь собрано всё, что относится к Linux-стороне проекта. Windows-версия
остаётся нетронутой: исходники общие, различия закрыты `#[cfg]`.

## Что это

Ретранслятор (`relay`) — единственный компонент Rat Next, который штатно
живёт на Linux: сервер сводит агента и вьюера по ID и перекачивает байты
сессии, не читая их. Агент и вьюер остаются Windows-приложениями (DXGI,
SendInput, ConPTY, служба SCM) и здесь не собираются.

Сетевой протокол не менялся, поэтому Linux-relay полностью совместим с
существующими Windows-клиентами.

## Состав папки

| Файл | Назначение |
|---|---|
| `PORTING.md` | Полное описание порта: что менялось, как собирать, что проверено |
| `rat-relay-control.sh` | Меню управления службой + команды для консоли |
| `rat-relay-control.desktop` | Шаблон ярлыка на рабочий стол |
| `install-linux-shortcut.sh` | Установщик ярлыка: раскладывает всё в `~/bin` и `~/Desktop` |
| `rat-relay.service` | Готовый unit для ручной установки (бинарник в `/usr/bin`) |
| `relay-smoke.py` | Функциональный тест протокола (9 проверок) |
| `cc-wrapper.sh` | Обёртка-линкер для машин без C-тулчейна |

Rust-код живёт там же, где и раньше: `bins/relay/src/` (общий для обеих ОС)
и `bins/relay/src/*_linux.rs` (Linux-реализации модулей).

## Быстрый старт

```bash
# 1. Сборка (сервер без GUI).
cargo build --release -p rat-relay --no-default-features

# 2. Ярлык и установка в ~/bin.
linux/install-linux-shortcut.sh

# 3. Установка службы systemd.
~/bin/rat-relay-control install
```

Дальше — двойной клик по ярлыку «Rat Next relay — управление» на рабочем
столе, либо `~/bin/rat-relay-control status`.

Подробности, включая сборку на машине без `gcc`, — в [PORTING.md](PORTING.md).

## Управление без ярлыка

```bash
~/bin/rat-relay-control status      # состояние, порт, PID, память
~/bin/rat-relay-control start       # запустить службу
~/bin/rat-relay-control stop        # остановить
~/bin/rat-relay-control restart     # перезапустить
~/bin/rat-relay-control log         # журнал (journalctl -f)
~/bin/rat-relay-control foreground  # на переднем плане, Ctrl+C — стоп
~/bin/rat-relay-control install     # поставить службу
~/bin/rat-relay-control uninstall   # снять службу
~/bin/rat-relay-control info        # где что лежит
```

## Проверка

```bash
# Адрес relay, порт по умолчанию 8889.
python3 linux/relay-smoke.py 193.7.203.222:8889
```

Тест проверяет полный цикл: `WHO` → `ERR OFFLINE`, `REG` → `OK`,
`WHO` → `OK`, `CONNECT` → `OK` + `PAIRED`, пересылку байтов в обе стороны,
бинарные данные с нулями и переводами строк, `CONNECT` к неизвестному ID.

## Где что лежит на машине

| Что | Путь |
|---|---|
| Бинарник (его запускает служба) | `~/bin/rat-relay` |
| Скрипт управления | `~/bin/rat-relay-control` |
| Unit службы | `/etc/systemd/system/rat-relay.service` |
| Конфиг (необязателен) | `/etc/rat-next/relay.json` |
| Журнал | `journalctl -u rat-relay` |
| Рабочий каталог службы | `/var/lib/rat-relay` |
| Замок единственного экземпляра | `$XDG_RUNTIME_DIR/rat-next-relay-*.lock` |

## Важно понимать

**Relay не шифрует.** Он перекачивает непрозрачные байты: экран, файлы и
нажатия клавиш идут через него открытым текстом. Для пилота в доверенном
контуре это допустимо; для внешнего использования нужен TLS/Noise (фаза P4
в `docs/ARCHITECTURE_DESIGN_v1.1.md`).

**GUI не собирается на сервере.** Фича `gui` требует GTK3 и
`libappindicator3-dev`, а также графической сессии. На сервере relay
работает headless — это штатный режим, а не урезанный.
