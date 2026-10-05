//! Автозапуск relay при входе пользователя через Планировщик задач.
//!
//! Задача запускает `rat-relay.exe --tray` — relay стартует сразу в трее.

const TASK: &str = "Rat Next relay";

/// Установлена ли задача автозапуска.
#[allow(dead_code)]
pub fn is_installed() -> bool {
    std::process::Command::new("schtasks")
        .args(["/Query", "/TN", TASK])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Создать (или пересоздать) задачу: запуск при входе, сразу в трей.
pub fn install() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let tr = format!("\"{}\" --tray", exe.display());
    let out = std::process::Command::new("schtasks")
        .args([
            "/Create", "/F", "/SC", "ONLOGON", "/TN", TASK, "/TR", &tr, "/RL", "LIMITED",
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
    let out = std::process::Command::new("schtasks")
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
