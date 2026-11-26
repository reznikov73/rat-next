#!/bin/bash
# Rat Next relay — управление службой.
#
# Запуск без аргументов открывает меню. С аргументом выполняет одно действие:
#   status | start | stop | restart | log | foreground | install | uninstall | info
#
# Служба systemd держит порт 8889 без окна. Режим "foreground" (окно relay)
# останавливает службу: они делят замок единственного экземпляра, и два
# процесса не уживутся.

set -u

UNIT="rat-relay"
UNIT_FILE="rat-relay.service"
UNIT_PATH="/etc/systemd/system/$UNIT_FILE"
PORT="${RAT_RELAY_PORT:-8889}"

# Бинарник рядом со скриптом: ~/bin/rat-relay при установке в ~/bin,
# иначе — собранный в репозитории.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -x "$SCRIPT_DIR/rat-relay" ]; then
	BIN="$SCRIPT_DIR/rat-relay"
elif [ -x "$SCRIPT_DIR/../../target/release/rat-relay" ]; then
	BIN="$SCRIPT_DIR/../../target/release/rat-relay"
else
	BIN="$SCRIPT_DIR/rat-relay"
fi

# Цвета только если вывод в терминал.
if [ -t 1 ]; then
	C_OK=$'\033[32m'; C_ERR=$'\033[31m'; C_WARN=$'\033[33m'
	C_DIM=$'\033[90m'; C_B=$'\033[1m'; C_0=$'\033[0m'
else
	C_OK=; C_ERR=; C_WARN=; C_DIM=; C_B=; C_0=
fi

ok()   { printf '%s%s%s\n' "$C_OK" "$1" "$C_0"; }
err()  { printf '%s%s%s\n' "$C_ERR" "$1" "$C_0"; }
warn() { printf '%s%s%s\n' "$C_WARN" "$1" "$C_0"; }
dim()  { printf '%s%s%s\n' "$C_DIM" "$1" "$C_0"; }

# run_root выполняет команду от root, спрашивая пароль в терминале при
# необходимости. Без терминала (запуск из скрипта) работает только
# NOPASSWD-правило sudo, иначе честно говорит об этом.
run_root() {
	if [ -t 0 ]; then
		sudo "$@"
	else
		sudo -n "$@" || return $?
	fi
}

# need_sudo проверяет, что управление вообще возможно.
need_sudo() {
	if sudo -n true 2>/dev/null; then
		return 0
	fi
	if [ -t 0 ]; then
		# Пароль спросит сам sudo при первом действии.
		return 0
	fi
	err "Нет доступа к systemctl без пароля и нет терминала для его ввода."
	dim "Запустите из терминала либо настройте /etc/sudoers.d/rat-relay."
	return 1
}

# unit_present: systemd знает про юнит.
unit_present() {
	[ -f "$UNIT_PATH" ]
}

# state: активное состояние службы.
state() {
	local out
	out="$(systemctl is-active "$UNIT_FILE" 2>/dev/null)"
	case "$out" in
		active|activating)             echo active ;;
		inactive|failed|deactivating)  echo inactive ;;
		*)                             echo unknown ;;
	esac
}

enabled() {
	systemctl is-enabled "$UNIT_FILE" 2>/dev/null || echo "—"
}

port_busy() {
	ss -tln 2>/dev/null | grep -qE "[:.]$PORT[[:space:]]"
}

port_owner() {
	local pid
	pid="$(ss -ltnpH "sport = :$PORT" 2>/dev/null | grep -o 'pid=[0-9]*' | head -1 | cut -d= -f2)"
	if [ -n "${pid:-}" ]; then
		printf 'PID %s (%s)' "$pid" "$(ps -o comm= -p "$pid" 2>/dev/null || echo '?')"
	else
		printf 'владелец скрыт (нужен root)'
	fi
}

main_pid() {
	systemctl show "$UNIT_FILE" -p MainPID --value 2>/dev/null
}

action_status() {
	printf '%sСлужба%s\n' "$C_B" "$C_0"

	if ! unit_present; then
		warn "  состояние:  не установлена (нет $UNIT_FILE)"
		dim  "  Установить: пункт 6 меню или --install-service"
	else
		case "$(state)" in
			active)   ok   "  состояние:  active (работает)" ;;
			inactive) warn "  состояние:  inactive (остановлена)" ;;
			*)        err  "  состояние:  неизвестно" ;;
		esac
		printf '  автозапуск: %s\n' "$(enabled)"
	fi

	printf '%sПроверки%s\n' "$C_B" "$C_0"
	if port_busy; then
		ok "  порт $PORT: занят — $(port_owner)"
	else
		warn "  порт $PORT: свободен"
	fi

	local pid
	pid="$(main_pid)"
	if [ -n "${pid:-}" ] && [ "$pid" != "0" ]; then
		printf '  PID:        %s\n' "$pid"
		local mem
		mem="$(ps -o rss= -p "$pid" 2>/dev/null | awk '{printf "%.0f", $1/1024}')"
		[ -n "${mem:-}" ] && printf '  память:     %s МБ\n' "$mem"
	fi

	printf '  бинарник:   %s\n' "$BIN"
	if [ ! -x "$BIN" ]; then
		err "              файл не найден или не исполняемый"
	fi
}

action_start() {
	if ! unit_present; then
		err "Служба не установлена."
		dim "Сначала установите: $0 install"
		return 1
	fi
	if [ "$(state)" = "active" ]; then
		warn "Служба уже работает."
		return 0
	fi
	need_sudo || return 1

	if ! run_root systemctl start "$UNIT_FILE"; then
		err "Не удалось запустить службу."
		return 1
	fi

	sleep 1
	if port_busy; then
		ok "Служба запущена. Порт $PORT слушается."
	else
		warn "Служба запущена, но порт $PORT не занят."
		dim "Журнал: $0 log"
		return 1
	fi
}

action_stop() {
	if ! unit_present; then
		warn "Служба не установлена."
		return 0
	fi
	if [ "$(state)" != "active" ]; then
		warn "Служба не работает."
		return 0
	fi
	need_sudo || return 1

	if run_root systemctl stop "$UNIT_FILE"; then
		ok "Служба остановлена. Подключения клиентов невозможны."
	else
		err "Не удалось остановить службу."
		return 1
	fi
}

action_restart() {
	if ! unit_present; then
		err "Служба не установлена."
		return 1
	fi
	need_sudo || return 1

	if ! run_root systemctl restart "$UNIT_FILE"; then
		err "Не удалось перезапустить службу."
		return 1
	fi
	sleep 1
	if port_busy; then
		ok "Служба перезапущена."
	else
		warn "Перезапущена, но порт $PORT не занят. Журнал: $0 log"
		return 1
	fi
}

action_install() {
	if [ ! -x "$BIN" ]; then
		err "Не найден бинарник: $BIN"
		return 1
	fi
	if unit_present; then
		warn "Служба уже установлена."
		dim "Переустановить: $0 uninstall, затем $0 install"
		return 0
	fi
	need_sudo || return 1

	dim "Устанавливаю службу (потребуется пароль sudo)…"
	if "$BIN" --install-service; then
		ok "Служба установлена и запущена."
		sleep 1
		action_status
	else
		err "Установка не удалась. Смотрите сообщение выше."
		return 1
	fi
}

action_uninstall() {
	if ! unit_present; then
		warn "Служба не установлена."
		return 0
	fi
	need_sudo || return 1

	dim "Удаляю службу…"
	if "$BIN" --uninstall-service; then
		ok "Служба удалена."
	else
		err "Удаление не удалось. Смотрите сообщение выше."
		return 1
	fi
}

action_log() {
	if ! unit_present; then
		err "Служба не установлена — журнала нет."
		return 1
	fi
	dim "Журнал службы (Ctrl+C — выход)"
	run_root journalctl -u "$UNIT_FILE" -f -n 40
}

# foreground: то же, что окно в POSCloud, но relay на этой машине собирается
# без GUI (нет графической сессии). Поэтому режим запускает сервер в текущем
# терминале: видно журнал, Ctrl+C останавливает.
action_foreground() {
	if [ ! -x "$BIN" ]; then
		err "Не найден бинарник: $BIN"
		return 1
	fi
	if [ "$(state)" = "active" ]; then
		dim "Останавливаю службу, чтобы освободить порт и замок…"
		need_sudo || return 1
		run_root systemctl stop "$UNIT_FILE" || {
			err "Не удалось остановить службу."
			return 1
		}
	fi
	if port_busy; then
		err "Порт $PORT уже занят: $(port_owner)"
		return 1
	fi
	dim "Запускаю relay на переднем плане (Ctrl+C — остановить)…"
	echo
	"$BIN" --listen "0.0.0.0:$PORT"
	echo
	dim "Relay завершён. Вернуться к службе: $0 start"
}

action_info() {
	cat <<INFO
Rat Next relay — где что лежит

  Бинарник     $BIN
  Юнит         $UNIT_PATH
  Настройки    /etc/rat-next/relay.json (или ~/.config/rat-next/relay.json)
  Журнал       journalctl -u $UNIT_FILE
  Порт         $PORT/tcp
  Замок        \$XDG_RUNTIME_DIR/rat-next-relay-*.lock (или /tmp)
  Документация linux/PORTING.md

  Relay пересылает трафик без шифрования: содержимое сессии видно на сервере.
  Слушать 0.0.0.0:$PORT стоит только в доверенном контуре.
INFO
}

menu() {
	while true; do
		printf '\n%s Rat Next relay%s\n' "$C_B" "$C_0"
		action_status
		printf '\n'
		printf '  1) Запустить службу\n'
		printf '  2) Остановить службу\n'
		printf '  3) Перезапустить службу\n'
		printf '  4) Открыть журнал службы\n'
		printf '  5) Запустить на переднем плане (служба будет остановлена)\n'
		printf '  6) Установить службу\n'
		printf '  7) Удалить службу\n'
		printf '  8) Где что лежит\n'
		printf '  0) Выход\n'
		printf '\nВыбор: '
		read -r choice || return 0
		case "$choice" in
			1) action_start ;;
			2) action_stop ;;
			3) action_restart ;;
			4) action_log; return 0 ;;
			5) action_foreground; return 0 ;;
			6) action_install ;;
			7) action_uninstall ;;
			8) action_info ;;
			0|q|Q) return 0 ;;
			*) warn "Нет такого пункта." ;;
		esac
	done
}

case "${1:-}" in
	status)     action_status ;;
	start)      action_start ;;
	stop)       action_stop ;;
	restart)    action_restart ;;
	log)        action_log ;;
	foreground) action_foreground ;;
	install)    action_install ;;
	uninstall)  action_uninstall ;;
	info)       action_info ;;
	"")         menu ;;
	*)
		err "Неизвестная команда: $1"
		printf 'Использование: %s [status|start|stop|restart|log|foreground|install|uninstall|info]\n' "$(basename "$0")"
		exit 1
		;;
esac
