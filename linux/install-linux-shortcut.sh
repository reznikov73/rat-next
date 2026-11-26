#!/bin/bash
# Устанавливает ярлык «Rat Next relay — управление» на рабочий стол.
#
# Что делает:
#   1. копирует собранный бинарник в ~/bin/rat-relay (стабильный путь: служба
#      не должна зависеть от каталога target/ в репозитории);
#   2. копирует скрипт управления в ~/bin/rat-relay-control;
#   3. создаёт ~/Desktop/rat-relay-control.desktop и дублирует его в
#      ~/.local/share/applications (чтобы ярлык был виден и в меню).
#
# Запуск:  linux/install-linux-shortcut.sh
# Удалить: linux/install-linux-shortcut.sh --uninstall

set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
BIN_SRC="$REPO/target/release/rat-relay"
BIN_DST="$HOME/bin/rat-relay"
CTL_SRC="$REPO/linux/rat-relay-control.sh"
CTL_DST="$HOME/bin/rat-relay-control"
DESKTOP_FILE="rat-relay-control.desktop"

DESKTOP_DIR="$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")"
APPS_DIR="$HOME/.local/share/applications"

uninstall() {
	echo "Удаляю ярлык…"
	rm -f "$DESKTOP_DIR/$DESKTOP_FILE" "$APPS_DIR/$DESKTOP_FILE"
	echo "Готово. Бинарник и скрипт управления оставлены в ~/bin:"
	echo "  $BIN_DST"
	echo "  $CTL_DST"
	echo "Чтобы удалить и их: rm -f \"$BIN_DST\" \"$CTL_DST\""
}

if [ "${1:-}" = "--uninstall" ]; then
	uninstall
	exit 0
fi

# 1. Бинарник.
if [ ! -x "$BIN_SRC" ]; then
	echo "Не найден бинарник: $BIN_SRC" >&2
	echo "Сначала соберите: cargo build --release -p rat-relay --no-default-features" >&2
	exit 1
fi
mkdir -p "$HOME/bin"
install -m 755 "$BIN_SRC" "$BIN_DST"
echo "бинарник:  $BIN_DST"

# 2. Скрипт управления.
install -m 755 "$CTL_SRC" "$CTL_DST"
echo "управление: $CTL_DST"

# 3. Ярлык. Пути подставляются абсолютные: xfce4-terminal запускает команду
#    через shell, но полагаться на раскрытие `~` в чужом окружении не стоит.
mkdir -p "$DESKTOP_DIR" "$APPS_DIR"

TERMINAL=""
for t in xfce4-terminal gnome-terminal konsole x-terminal-emulator xterm; do
	if command -v "$t" >/dev/null 2>&1; then
		TERMINAL="$t"
		break
	fi
done

if [ -z "$TERMINAL" ]; then
	echo "Не найден ни один терминал (xfce4-terminal/gnome-terminal/…)" >&2
	echo "Ярлык не создан: без терминала меню управления не показать." >&2
	exit 1
fi

case "$TERMINAL" in
	xfce4-terminal)
		EXEC="$TERMINAL --title=\"Rat Next relay — управление\" --command=\"$CTL_DST\""
		;;
	gnome-terminal)
		EXEC="$TERMINAL --title=Rat Next relay — управление -- $CTL_DST"
		;;
	konsole)
		EXEC="$TERMINAL -p tabtitle=Rat Next relay — управление -e $CTL_DST"
		;;
	*)
		EXEC="$TERMINAL -e $CTL_DST"
		;;
esac

make_desktop() {
	cat <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=Rat Next relay — управление
Name[ru]=Rat Next relay — управление
Comment=Запуск, остановка и журнал службы Rat Next relay
Comment[ru]=Запуск, остановка и журнал службы Rat Next relay
Exec=$EXEC
Icon=network-server
Terminal=false
Categories=Network;
Keywords=rat;relay;ratnext;service;ретранслятор;
StartupNotify=false
EOF
}

make_desktop > "$DESKTOP_DIR/$DESKTOP_FILE"
chmod +x "$DESKTOP_DIR/$DESKTOP_FILE"
make_desktop > "$APPS_DIR/$DESKTOP_FILE"
chmod +x "$APPS_DIR/$DESKTOP_FILE"

echo "ярлык:     $DESKTOP_DIR/$DESKTOP_FILE"

# Проверка синтаксиса, если доступен валидатор.
if command -v desktop-file-validate >/dev/null 2>&1; then
	if desktop-file-validate "$DESKTOP_DIR/$DESKTOP_FILE"; then
		echo "проверка:  desktop-file-validate — без замечаний"
	else
		echo "проверка:  desktop-file-validate нашёл замечания (см. выше)" >&2
	fi
fi

echo
echo "Готово. На рабочем столе появился «Rat Next relay — управление»."
echo "Если значок выглядит как текстовый файл — нажмите на нём правой кнопкой"
echo "и выберите «Разрешить запуск» (так требует XFCE/GNOME для новых ярлыков)."
