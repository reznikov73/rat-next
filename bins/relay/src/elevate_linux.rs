//! Повышение прав — Linux (polkit/`sudo`).
//!
//! Заменяет `elevate.rs` с `ShellExecuteW("runas")` и запросом UAC.
//!
//! В Windows интерфейс перезапускает сам себя с флагом, чтобы UAC показал
//! запрос. В Linux аналог — `pkexec`, который показывает диалог polkit, но
//! ему нужна установленная policy-раскладка; поэтому основной путь —
//! честное сообщение «выполните команду с sudo», а `pkexec` пробуем, если
//! он есть в системе.

use std::process::Command;

/// Перезапустить эту команду с повышенными правами.
///
/// `flag` — аргумент командной строки (например, `--install-service`).
/// `Ok(())` — запрос отправлен и выполнен успешно.
pub fn relaunch(flag: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("путь к exe: {e}"))?;

    // pkexec есть — используем его: это графический аналог UAC.
    if Command::new("pkexec")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        let out = Command::new("pkexec")
            .arg(&exe)
            .arg(flag)
            .output()
            .map_err(|e| format!("pkexec: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // Отмена диалога polkit — не ошибка программы, но и не успех.
        return Err(if err.is_empty() {
            format!("pkexec завершился с кодом {}", out.status)
        } else {
            err
        });
    }

    // pkexec нет — подсказываем точную команду.
    Err(format!(
        "для этого действия нужны права root. Выполните:\n  sudo {} {}",
        exe.display(),
        flag
    ))
}

/// `true`, если процесс уже с правами root.
pub fn is_elevated() -> bool {
    // geteuid без libc: читаем /proc/self/status (строка Uid:).
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2))
                .and_then(|v| v.parse::<u32>().ok())
        })
        == Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevated_matches_proc_status() {
        // Согласованность: если euid 0 — /proc говорит 0.
        let from_proc = std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|t| {
                t.lines()
                    .find(|l| l.starts_with("Uid:"))
                    .and_then(|l| l.split_whitespace().nth(2))
                    .and_then(|v| v.parse::<u32>().ok())
            });
        assert_eq!(is_elevated(), from_proc == Some(0));
    }
}
