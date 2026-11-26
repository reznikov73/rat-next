# Relay под Linux: порт и запуск

Документ описывает, как Windows-версия `rat-relay` перенесена на Linux, что
именно изменилось в коде и как поднять сервис на этой машине.

## 1. Что было и что стало

`bins/relay` — сервер-посредник: сводит агента и вьюера по ID и перекачивает
байты сессии, не читая их. Сетевого ядра порт не касался: протокол `REG /
CONNECT / WHO` и `pump()` на `io::copy` переносятся один в один, поэтому
Linux-relay полностью совместим с существующими Windows-агентами и вьюерами.

Изменения — только в платформенной обвязке и в способе сборки:

| Windows | Linux |
|---|---|
| Служба SCM (`service.rs`, 830 строк Win32) | systemd unit `rat-relay.service` |
| Надзор за GUI-хелпером в активной сессии | не нужен: нет сессии 0 |
| Автозапуск через `schtasks` | systemd (сервер) / XDG autostart (окно) |
| UAC через `ShellExecuteW("runas")` | `pkexec`, иначе — подсказка с `sudo` |
| `netstat` + `tasklist` + `taskkill` | `ss` + `/proc/<pid>/comm` + `kill` |
| `relay.json` рядом с exe | `/etc/rat-next/relay.json`, затем `~/.config` |
| `relay.log` рядом с exe | journald (`journalctl -u rat-relay`) |
| Окно egui + трей обязательно | фича `gui` (по умолчанию вкл., на сервере выкл.) |
| `WGPU_BACKEND=dx12` | бэкенд выбирает wgpu (Vulkan/GL) |
| Иконка в exe через `winresource` | не нужна (build-скрипт выходит сразу) |

## 2. Структура после порта

```
bins/relay/
  Cargo.toml          # фича gui; winresource только под cfg(windows)
  build.rs            # ранний выход вне Windows; вызов winresource под cfg
  src/
    main.rs           # CLI, диспетчеризация, headless-режим, сигналы
    core.rs           # без изменений: TCP, pairing, pump
    icon.rs           # без изменений
    settings.rs       # путь конфига различается по платформе
    ui.rs             # без изменений в логике; DX12 только под cfg(windows)
    tray.rs           # без изменений (нужен GTK при сборке с gui)
    service.rs        # Windows: SCM (не менялся)
    service_linux.rs  # Linux: systemd (новый)
    port.rs           # Windows: netstat/taskkill (не менялся)
    port_linux.rs     # Linux: ss/lsof/kill (новый)
    autostart.rs      # Windows: schtasks (+ добавлен install())
    autostart_linux.rs# Linux: XDG autostart (новый)
    elevate.rs        # Windows: UAC (не менялся)
    elevate_linux.rs  # Linux: pkexec/sudo (новый)
```

Модули с одинаковыми именами подключаются через `#[cfg]` + `#[path]`, поэтому
`main.rs` и `ui.rs` не знают, под какой ОС собраны.

## 3. Сборка на этой машине

Системного C-тулчейна нет (нет `gcc`/`clang`, нет `crt1.o` и `libc.so` из
`-dev`), поэтому линковка идёт через `rust-lld` из состава rustup и sysroot,
собранный из пакетов без `sudo`. Подготовка:

```bash
# 1. Sysroot: только заголовки и crt-объекты, без установки в систему.
mkdir -p ~/sysroot && cd /tmp
B=http://archive.ubuntu.com/ubuntu
for f in pool/main/g/glibc/libc6-dev_2.35-0ubuntu3_amd64.deb \
         pool/main/g/gcc-12/libgcc-12-dev_12-20220319-1ubuntu1_amd64.deb \
         pool/main/g/gcc-12/gcc-12-base_12-20220319-1ubuntu1_amd64.deb; do
  curl -O "$B/$f" && dpkg -x "$(basename "$f")" ~/sysroot
done

# 2. Линкер-скрипты glibc ссылаются на абсолютные пути — заменяем на симлинки.
S=~/sysroot/usr/lib/x86_64-linux-gnu
for l in libc.so.6 libm.so.6 libmvec.so.1 libpthread.so.0 librt.so.1 libdl.so.2 libutil.so.1; do
  ln -sf /lib/x86_64-linux-gnu/$l $S/$l
done
ln -sf /lib/x86_64-linux-gnu/libgcc_s.so.1 $S/libgcc_s.so
printf 'OUTPUT_FORMAT(elf64-x86-64)\nGROUP ( libm.so.6 AS_NEEDED ( libmvec.so.1 ) )\n' > $S/libm.so
printf 'OUTPUT_FORMAT(elf64-x86-64)\nGROUP ( libc.so.6 libc_nonshared.a AS_NEEDED ( /lib64/ld-linux-x86-64.so.2 ) )\n' > $S/libc.so
mkdir -p ~/sysroot/lib64 && ln -sf /lib64/ld-linux-x86-64.so.2 ~/sysroot/lib64/

# 3. Сборка headless-бинаря.
export PATH=$HOME/.cargo/bin:$PATH
export RUSTFLAGS="-C linker=$PWD/linux/cc-wrapper.sh"
cargo build --release -p rat-relay --no-default-features
```

`linux/cc-wrapper.sh` — не «костыль на будущее», а рабочий линкер: он
подставляет `Scrt1/crti/crtn` и `crtbegin/crtend`, которых rustc ждёт от `cc`,
и вызывает `rust-lld -flavor gnu`. При появлении настоящего `gcc` обёртка
не нужна: `cargo build` заработает сам.

**Если появится `sudo`:** правильнее поставить штатный тулчейн
(`sudo apt install build-essential pkg-config libgtk-3-dev libappindicator3-dev`)
и собирать без обёртки. Она нужна только чтобы проверить порт на машине,
где пакеты ставить нельзя.

## 4. Запуск

### Вариант А: systemd (рекомендуется)

```bash
# Установка: создаёт /etc/systemd/system/rat-relay.service и запускает.
sudo ./target/release/rat-relay --install-service

# Состояние и журнал.
systemctl status rat-relay
journalctl -u rat-relay -f

# Остановка / удаление.
sudo ./target/release/rat-relay --stop-service
sudo ./target/release/rat-relay --uninstall-service
```

### Вариант А′: ярлык на рабочем столе

Если не хочется помнить команды — есть графическое управление, устроенное
так же, как `poscloud-control.desktop` на этой машине:

```bash
linux/install-linux-shortcut.sh              # поставить ярлык
linux/install-linux-shortcut.sh --uninstall  # убрать ярлык
```

Скрипт раскладывает бинарник и управление в `~/bin` и создаёт
`~/Desktop/rat-relay-control.desktop` (плюс копию в
`~/.local/share/applications`, чтобы ярлык был в меню). Двойной клик открывает
терминал с меню: запуск, остановка, перезапуск, журнал, запуск на переднем
плане, установка/удаление службы, справка о раскладке файлов.

Тот же скрипт работает и из консоли — без меню:

```bash
~/bin/rat-relay-control status
~/bin/rat-relay-control start
~/bin/rat-relay-control install
~/bin/rat-relay-control log
```

Пароль `sudo` спрашивается прямо в этом терминале: ярлык открывает настоящий
TTY, поэтому установка службы из графики работает без ручной настройки
sudoers. Если запускать без терминала, используется `sudo -n` — чтобы
процесс не повис навсегда в ожидании ввода.

Готовый unit для ручной установки лежит в `linux/rat-relay.service` — его
можно поставить и вручную (он содержит изоляцию: `DynamicUser`,
`ProtectSystem=strict`, `PrivateTmp`, `RestrictAddressFamilies`). Версия,
которую генерирует `--install-service`, учитывает местную раскладку: путь
экранируется как `\x20` (каталог содержит пробел), а служба запускается от
владельца бинарника — иначе `/home/<user>` с правами 750 не пустил бы её
внутрь. Переопределяется переменной `RAT_RELAY_USER`.

### Вариант Б: на переднем плане

```bash
./target/release/rat-relay --listen 0.0.0.0:8889
```

Печатает `RELAY_LISTENING <адрес>` и работает до `SIGTERM`/`Ctrl+C`, затем
`RELAY_STOPPED`. Именно этот режим использует systemd.

### Проверка

```bash
./target/release/rat-relay --smoke=5          # поднять порт, 5 секунд, выйти
python3 linux/relay-smoke.py 127.0.0.1:8889 # полный тест протокола
```

`linux/relay-smoke.py` эмулирует настоящих клиентов: `WHO` → `ERR OFFLINE`,
`REG` → `OK`, `WHO` → `OK`, `CONNECT` → `OK` + `PAIRED`, пересылка байтов в обе
стороны, бинарные данные с нулями и переводами строк, `CONNECT` к
неизвестному ID → `ERR OFFLINE`. На этой машине проходит 9 из 9.

### Конфигурация

`/etc/rat-next/relay.json` (или `~/.config/rat-next/relay.json`):

```json
{
  "public_addr": "194.37.254.233:8889",
  "listen_addr": "0.0.0.0:8889"
}
```

`public_addr` — информационное поле для клиентов, `listen_addr` — что слушать.
Приоритет: флаг `--listen` → файл → значения по умолчанию.

## 5. Что важно понимать

**Relay не шифрует.** Он перекачивает байты как есть: экран, файлы и нажатия
клавиш идут через него открытым текстом. Порт это не меняет. Если сервер
смотрит в интернет на `0.0.0.0:8889`, наружу выставлен незашифрованный канал,
и `WHO` работает как оракул присутствия по ID. Для пилота в собственном
контуре это допустимо, для чего-то большего — нужен TLS/Noise (фаза P4 в
`docs/ARCHITECTURE_DESIGN_v1.1.md`).

**Двойной запуск закрыт.** `crates/session/src/single_instance.rs` на Linux
раньше был заглушкой (`try_acquire` всегда `true`), из-за чего два процесса
могли драться за порт. Теперь используется `flock` на файл в
`$XDG_RUNTIME_DIR` (или `/tmp`) с именем, привязанным к пути exe: копии в
разных папках не конфликтуют, повторный запуск того же файла получает отказ
(код возврата 2).

**SIGTERM обрабатывается.** Первая версия порта завершалась только по
`SIGKILL`: главный поток висел в блокирующем `recv()`, а `std` после `EINTR`
прозрачно повторяет вызов, поэтому сигнал терялся. Сейчас приём идёт через
`recv_timeout(100 мс)` с проверкой атомарного флага — `systemctl stop`
завершает процесс за доли секунды, печатая `RELAY_STOPPED`.

**GUI на этой машине не запустится.** `DISPLAY`, `WAYLAND_DISPLAY` и
`XDG_SESSION_TYPE` пусты — графической сессии нет. Сборка с фичей `gui`
требует GTK3 и `libappindicator3-dev`, которых тоже нет. Поэтому headless —
не компромисс, а единственный осмысленный вариант; окно собирается на машине
с рабочим столом тем же кодом (`--features gui`).

## 6. Проверенное состояние

- `cargo build --release -p rat-relay --no-default-features` — успешно,
  бинарник 871 КБ, ELF x86-64.
- `cargo test -p rat-relay --no-default-features` — 12 из 12, включая родной
  тест `core::tests::pair_pumps_bytes`.
- `cargo test -p rat-session` — 27 из 27 (всего 39 тестов зелёные).
- `linux/relay-smoke.py` на debug и на release — 9 из 9.
- `cargo clippy -p rat-relay --no-default-features -- -D warnings` — чисто.
- `cargo fmt --all --check` — чисто.
- `SIGTERM` → `RELAY_STOPPED`, код возврата 0 (проверено несколько раз,
  в том числе на release-бинаре).
- Повторный запуск → `relay: экземпляр уже запущен`, код 2.
- `desktop-file-validate` на установленном ярлыке — без замечаний.
- Ярлык открывает меню; `status`/`install`/`info` работают из консоли.
- Под псевдотерминалом `--install-service` доводит до `[sudo] пароль для`,
  то есть установка из ярлыка возможна без настройки sudoers.

Без проверки остаётся только сам факт ввода пароля и старт службы: у агента
его нет, а подставлять чужой пароль в тест — плохая идея.

Объём изменений: +400 строк в существующих файлах (из них 187 — `main.rs`
с headless-режимом и обработкой сигналов) и четыре новых файла-двойника
общим объёмом ~600 строк вместо 1 031 строки Windows-обвязки:
`service.rs` 830 → `service_linux.rs` 268, `port.rs` 105 → `port_linux.rs` 200,
`autostart.rs` 45 → `autostart_linux.rs` 88, `elevate.rs` 51 → `elevate_linux.rs` 83.

## 7. Что не проверено

- **Сборка под Windows.** На этой машине нет MSVC-линкера и Windows SDK,
  поэтому Windows-путь проверен только статически: все обращения к `ui::`
  находятся внутри `#[cfg(all(windows, feature = "gui"))]`, `autostart::install()`
  добавлен в Windows-версию, `run_service()` есть в обеих реализациях.
  Перед мержем стоит прогнать `cargo build --workspace` на Windows или в CI.
- **`--install-service` вживую.** Требует пароля `sudo`, которого у агента нет.
  Логика проверена юнит-тестом на текст unit-файла; сама установка — за
  владельцем.
- **Сборка с `--features gui` на Linux.** Блокируется отсутствием
  `libgtk-3-dev`/`libappindicator3-dev`; код (`ui.rs`, `tray.rs`) при этом
  не менялся, кроме `cfg(windows)` вокруг DX12.
- **Поведение под нагрузкой.** Проверены единичные сессии; лимитов на число
  подключений в `core.rs` нет ни в Windows-, ни в Linux-версии.
