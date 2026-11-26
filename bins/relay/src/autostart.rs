//! Автозапуск relay при входе пользователя через Планировщик задач.
//!
//! Задача запускает `rat-relay.exe --tray` — relay стартует сразу в трее.

const TASK: &str = "Rat Next relay";

/// `schtasks` без мелькания консольного окна.
fn schtasks() -> std::process::Command {
    let mut cmd = std::process::Command::new("schtasks");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Установлена ли задача автозапуска.
#[allow(dead_code)]
pub fn is_installed() -> bool {
    schtasks()
        .args(["/Query", "/TN", TASK])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Включить автозапуск: создать задачу «при входе пользователя».
///
/// Задача запускает тот же exe с `--tray`, то есть relay стартует свёрнутым
/// в трей, без окна поверх остальных приложений.
pub fn install() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("путь к exe: {e}"))?;
    let out = schtasks()
        .args([
            "/Create",
            "/F",
            "/SC",
            "ONLOGON",
            "/TN",
            TASK,
            "/TR",
            &format!("\"{}\" --tray", exe.display()),
        ])
        .output()
        .map_err(|e| format!("schtasks: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let err = if err.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            err
        };
        Err(err)
    }
}

/// Удалить задачу автозапуска.
pub fn remove() -> Result<(), String> {
    let out = schtasks()
        .args(["/Delete", "/F", "/TN", TASK])
        .output()
        .map_err(|e| format!("schtasks: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let err = if err.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            err
        };
        Err(err)
    }
}
