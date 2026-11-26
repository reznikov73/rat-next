//! Автозапуск relay при входе пользователя — Linux (XDG autostart).
//!
//! Заменяет `autostart.rs` с Планировщиком задач Windows (`schtasks`).
//!
//! Важно: это автозапуск **окна** relay при входе в графическую сессию.
//! Фоновый сервер правильнее ставить systemd-сервисом (`--install-service`),
//! поэтому `.desktop`-файл описывает именно GUI-режим и не конфликтует с
//! сервисом: если сервис установлен, interface подключается к нему.

use std::path::PathBuf;

/// Имя файла автозапуска.
const DESKTOP_FILE: &str = "rat-relay.desktop";

/// Каталог XDG autostart текущего пользователя.
fn autostart_dir() -> Option<PathBuf> {
    // XDG_CONFIG_HOME, иначе ~/.config
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            let home = std::env::var_os("HOME")?;
            PathBuf::from(home).join(".config")
        }
    };
    Some(base.join("autostart"))
}

/// Путь к `.desktop`-файлу автозапуска.
pub fn path() -> Option<PathBuf> {
    autostart_dir().map(|d| d.join(DESKTOP_FILE))
}

/// Установлен ли автозапуск.
pub fn is_installed() -> bool {
    path().map(|p| p.exists()).unwrap_or(false)
}

/// Включить автозапуск: создаёт `~/.config/autostart/rat-relay.desktop`.
pub fn install() -> Result<(), String> {
    let path = path().ok_or_else(|| "не удалось определить HOME".to_string())?;
    let dir = path
        .parent()
        .ok_or_else(|| "некорректный путь автозапуска".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let exe = std::env::current_exe().map_err(|e| format!("путь к exe: {e}"))?;

    // StartupNotify=false: окно показывается само; --tray просит стартовать
    // свёрнутым, как и в Windows-версии.
    let text = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Rat Next relay\n\
         Comment=Relay-сервер Rat Next: pairing по ID и пересылка байтов сессии\n\
         Exec=\"{exe}\" --tray\n\
         Icon=network-server\n\
         Terminal=false\n\
         StartupNotify=false\n\
         Categories=Network;\n",
        exe = exe.display()
    );

    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Выключить автозапуск: удаляет `.desktop`-файл, если он есть.
pub fn remove() -> Result<(), String> {
    let Some(path) = path() else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_path_ends_with_expected_name() {
        // HOME в тестах обычно задан; если нет — path() вернёт None, и это ок.
        if let Some(p) = path() {
            assert!(p.ends_with("autostart/rat-relay.desktop"));
        }
    }
}
