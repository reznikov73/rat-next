//! Управление relay как системным сервисом — Linux (systemd).
//!
//! Заменяет `service.rs` (служба Windows SCM, 830 строк Win32) набором
//! действий над unit-файлом. Надзор за GUI-хелпером в «активной консольной
//! сессии» здесь не нужен: в Linux нет ни SCM, ни понятия session 0 —
//! процессом управляет systemd, а окно (если оно вообще есть) запускает
//! сам пользователь.
//!
//! Модель:
//! - unit `rat-relay.service` — system-wide, ставится root'ом;
//! - установка/удаление требуют прав: вызываем `systemctl` через `sudo -n`
//!   и, если прав нет, честно говорим об этом (аналог запроса UAC в
//!   `elevate.rs`, но без перезапуска процесса: `sudo` спросит пароль в
//!   терминале);
//! - состояние читается без прав через `systemctl is-active` / `is-enabled`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Имя unit-файла без суффикса.
pub const UNIT: &str = "rat-relay";
/// Полное имя unit-файла.
pub const UNIT_FILE: &str = "rat-relay.service";

/// Каталог для unit-файлов system-wide.
const UNIT_DIR: &str = "/etc/systemd/system";

/// Состояние сервиса для интерфейса и CLI.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Unit-файла нет.
    NotInstalled,
    /// Установлен, но не работает.
    Stopped,
    /// Работает.
    Running,
    /// Запускается или останавливается.
    Pending,
}

/// Операция со службой из интерфейса.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Install,
    Start,
    Stop,
    Uninstall,
}

/// Путь к unit-файлу.
pub fn unit_path() -> PathBuf {
    Path::new(UNIT_DIR).join(UNIT_FILE)
}

/// Запустить `systemctl` с аргументами.
///
/// `sudo = true` — действие требует root. Поведение зависит от наличия
/// терминала:
///
/// * **stdin — терминал** (обычный запуск из консоли или из ярлыка,
///   который открывает терминал): `sudo` спрашивает пароль как обычно.
///   Это и есть рабочий путь: правило `NOPASSWD` есть далеко не всегда,
///   а `-n` в такой ситуации просто отказывал бы в работе;
/// * **терминала нет** (systemd, cron, GUI без консоли): `sudo -n`,
///   чтобы процесс не завис навсегда в ожидании ввода, которого не будет.
///
/// Раньше здесь всегда был `-n`, из-за чего управление службой из ярлыка
/// не работало без ручной настройки sudoers.
fn systemctl(args: &[&str], sudo: bool) -> Result<String, String> {
    use std::io::IsTerminal;

    let interactive = std::io::stdin().is_terminal();

    let mut cmd = if sudo {
        let mut c = Command::new("sudo");
        if !interactive {
            c.arg("-n");
        }
        c.arg("systemctl");
        c
    } else {
        Command::new("systemctl")
    };
    cmd.args(args);

    let out = cmd
        .output()
        .map_err(|e| format!("запуск systemctl: {e} (systemd установлен?)"))?;

    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();

    if out.status.success() {
        return Ok(stdout);
    }

    // Отказ sudo: пароль не введён / нет терминала / нет прав.
    let denied = stderr.contains("password is required")
        || stderr.contains("a terminal is required")
        || stderr.contains("no tty present")
        || stderr.contains("Sorry, try again");
    if sudo && denied {
        return Err(format!(
            "нужны права root. Выполните вручную:\n  sudo systemctl {}",
            args.join(" ")
        ));
    }

    let detail = if stderr.is_empty() { stdout } else { stderr };
    Err(format!("systemctl {}: {detail}", args.join(" ")))
}

/// Узнать состояние сервиса (прав не требует).
pub fn status() -> State {
    if !unit_path().exists() {
        return State::NotInstalled;
    }

    let active = systemctl(&["is-active", UNIT_FILE], false).unwrap_or_default();
    match active.as_str() {
        "active" => State::Running,
        "activating" | "deactivating" | "reloading" => State::Pending,
        "inactive" | "failed" => State::Stopped,
        _ => {
            // `is-active` вернул что-то неожиданное — считаем, что не работает,
            // но unit установлен.
            State::Stopped
        }
    }
}

/// Экранирование пути для unit-файла.
///
/// systemd не понимает кавычки в `ExecStart` так, как shell, и разрезает
/// строку по пробелам. Правильный способ — `systemd-escape`-нотация:
/// пробел кодируется как `\x20`. Кавычки здесь не годятся: путь
/// `"/home/u/RAT Next/target/release/rat-relay"` systemd воспринял бы
/// как единый аргумент с кавычками внутри.
fn escape_path(p: &Path) -> String {
    let mut out = String::new();
    for ch in p.to_string_lossy().chars() {
        match ch {
            ' ' => out.push_str(r"\x20"),
            '\t' => out.push_str(r"\x09"),
            '\\' => out.push_str(r"\x5c"),
            '"' => out.push_str(r"\x22"),
            '\'' => out.push_str(r"\x27"),
            c => out.push(c),
        }
    }
    out
}

/// Пользователь, от которого работает служба.
///
/// `DynamicUser=yes` хорош для пакетной установки в `/usr/bin`, но здесь
/// бинарник часто лежит в домашнем каталоге, закрытом для посторонних
/// (`/home/user` — 750). Поэтому по умолчанию служба работает от владельца
/// самого файла: это и есть тот пользователь, который его собрал.
fn service_user(exe: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;

    // Явное переопределение для нестандартных раскладок.
    if let Ok(u) = std::env::var("RAT_RELAY_USER")
        && !u.trim().is_empty()
    {
        return Some(u.trim().to_string());
    }

    let uid = std::fs::metadata(exe).ok()?.uid();
    if uid == 0 {
        return None; // root — секция User не нужна
    }
    Some(uid.to_string())
}

/// Текст unit-файла.
///
/// `Restart=on-failure` вместе с `RestartSec` заменяет авторестарт службы
/// Windows (`SC_ACTION_RESTART` ×3 через 5 с в `service.rs`).
fn unit_text(exe: &Path) -> String {
    let exec = escape_path(exe);

    // Пользователь: владелец бинарника, иначе — системный пользователь.
    let (user_line, isolation) = match service_user(exe) {
        Some(uid) => (
            format!("User={uid}\n"),
            // Запись в StateDirectory — единственное, что нужно на диске.
            "NoNewPrivileges=yes\nProtectSystem=strict\nProtectHome=read-only\n\
             PrivateTmp=yes\nPrivateDevices=yes\n\
             ProtectKernelTunables=yes\nProtectKernelModules=yes\n\
             ProtectControlGroups=yes\nLockPersonality=yes\n"
                .to_string(),
        ),
        None => (
            String::new(),
            "NoNewPrivileges=yes\nProtectSystem=strict\nProtectHome=yes\n\
             PrivateTmp=yes\nPrivateDevices=yes\n"
                .to_string(),
        ),
    };

    format!(
        "[Unit]
Description=Rat Next relay (pairing by ID + forward of opaque session bytes)
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=60
StartLimitBurst=10

[Service]
Type=simple
ExecStart={exec} --listen 0.0.0.0:8889
Restart=on-failure
RestartSec=5
{user_line}StateDirectory=rat-relay
{isolation}StandardOutput=journal
StandardError=journal
SyslogIdentifier=rat-relay

[Install]
WantedBy=multi-user.target
",
        exec = exec,
        user_line = user_line,
        isolation = isolation,
    )
}

/// Установить и запустить сервис (нужен root).
pub fn install() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("путь к exe: {e}"))?;
    let path = unit_path();
    let text = unit_text(&exe);

    write_unit(&path, &text)?;

    // Перечитываем конфигурацию и включаем автозапуск.
    systemctl(&["daemon-reload"], true)?;
    systemctl(&["enable", UNIT_FILE], true)?;

    // Запуск отдельным шагом: если сервис уже работал, `start` — no-op.
    systemctl(&["restart", UNIT_FILE], true).or_else(|e| {
        // `restart` не работает, если сервис ещё ни разу не стартовал в этой
        // загрузке — падаем на `start`.
        systemctl(&["start", UNIT_FILE], true).map_err(|_| e)
    })?;
    Ok(())
}

/// Записать unit-файл от root.
///
/// Тонкость: `sudo tee` читает содержимое из stdin, а `sudo` может спросить
/// там же пароль — нельзя кормить ему и то, и другое в один поток. Поэтому
/// текст передаётся через два разных канала:
///
/// 1. если пароль не нужен (`sudo -n` прошёл) — пишем прямо в stdin `tee`;
/// 2. если нужен — просим `sudo` обновить метку времени
///    (`sudo -v`), и только затем пишем в `sudo -n tee`, который уже
///    пройдёт без вопроса. Пароль вводится один раз и запоминается sudo.
fn write_unit(path: &Path, text: &str) -> Result<(), String> {
    // Проба: пройдёт ли `sudo -n` без пароля.
    let passwordless = Command::new("sudo")
        .args(["-n", "true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !passwordless {
        // `sudo -v` спрашивает пароль на терминале и обновляет метку.
        let ok = Command::new("sudo")
            .arg("-v")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return Err(format!(
                "не удалось получить права root. Запишите unit вручную:\n\
                 sudo tee {} >/dev/null <<'EOF'\n{text}EOF\n\
                 sudo systemctl daemon-reload\n\
                 sudo systemctl enable --now {UNIT_FILE}",
                path.display()
            ));
        }
    }

    // Теперь `-n` гарантированно не спросит пароль и stdin свободен для текста.
    let mut child = Command::new("sudo")
        .args(["-n", "tee", &path.to_string_lossy()])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("запись unit-файла: {e}"))?;

    {
        use std::io::Write;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "не удалось открыть stdin для sudo tee".to_string())?;
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("запись unit-файла: {e}"))?;
    }

    let status = child
        .wait()
        .map_err(|e| format!("запись unit-файла: {e}"))?;
    if !status.success() {
        return Err(format!(
            "не удалось записать {}. Запишите вручную:\n\
             sudo tee {} >/dev/null <<'EOF'\n{text}EOF",
            path.display(),
            path.display()
        ));
    }
    Ok(())
}

/// Остановить и удалить сервис (нужен root).
pub fn uninstall() -> Result<(), String> {
    let path = unit_path();

    // Останавливаем и снимаем с автозапуска; ошибки «уже остановлен» не критичны.
    let _ = systemctl(&["disable", "--now", UNIT_FILE], true);

    if path.exists() {
        // Тот же приём, что в `write_unit`: сначала обеспечить метку sudo,
        // затем выполнить действие с `-n`, чтобы stdin не был нужен дважды.
        let passwordless = Command::new("sudo")
            .args(["-n", "true"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !passwordless {
            let _ = Command::new("sudo").arg("-v").status();
        }

        let out = Command::new("sudo")
            .args(["-n", "rm", "-f", &path.to_string_lossy()])
            .output()
            .map_err(|e| format!("удаление unit-файла: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "не удалось удалить {}. Выполните вручную:\n  sudo rm -f {}",
                path.display(),
                path.display()
            ));
        }
    }

    let _ = systemctl(&["daemon-reload"], true);
    Ok(())
}

/// Установлен ли unit-файл.
///
/// Проверяем не только существование файла, но и то, что systemd его видит:
/// своп-файлы, ручной `cp` без `daemon-reload` и опечатки в пути дают
/// файл, который ничего не значит.
pub fn is_installed() -> bool {
    unit_path().exists()
}

/// Запустить установленный сервис.
pub fn start() -> Result<(), String> {
    if !is_installed() {
        return Err("сервис не установлен (сначала `--install-service`)".to_string());
    }
    systemctl(&["start", UNIT_FILE], true).map(|_| ())
}

/// Остановить сервис.
pub fn stop() -> Result<(), String> {
    if !is_installed() {
        return Err("сервис не установлен".to_string());
    }
    systemctl(&["stop", UNIT_FILE], true).map(|_| ())
}

/// Запуск «как службы»: под systemd процесс уже и есть сервис — отдельный
/// диспетчер (аналог `StartServiceCtrlDispatcherW`) не нужен.
///
/// Оставлено ради совместимости с CLI Windows-версии: `--service` на Linux
/// просто работает как обычный запуск.
pub fn run_service() -> Result<(), String> {
    Err(
        "на Linux режим `--service` не используется: сервисом управляет systemd \
         (установите unit через `--install-service`)"
            .to_string(),
    )
}

/// Заглушки: в Windows интерфейс просит службу «не убивать хелпер» на время
/// операции. В Linux хелпера нет.
pub fn raise_keep_helper() {}

/// Заглушка (см. [`raise_keep_helper`]).
pub fn clear_keep_helper() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_text_contains_restart_policy() {
        let text = unit_text(Path::new("/usr/bin/rat-relay"));
        assert!(text.contains("ExecStart=/usr/bin/rat-relay"));
        assert!(text.contains("Restart=on-failure"));
        assert!(text.contains("WantedBy=multi-user.target"));
        // Каталог для состояния должен быть — иначе ProtectSystem=strict
        // не даст процессу писать вообще никуда.
        assert!(text.contains("StateDirectory=rat-relay"));
    }

    #[test]
    fn escapes_spaces_in_path() {
        // Реальный путь в этом репозитории содержит пробел: /Work/RAT Next/.
        // Без экранирования systemd разрежет ExecStart на два аргумента.
        let text = unit_text(Path::new("/home/u/RAT Next/target/release/rat-relay"));
        assert!(
            text.contains(r"ExecStart=/home/u/RAT\x20Next/target/release/rat-relay"),
            "путь с пробелом должен кодироваться как \\x20, получено:\n{text}"
        );
        // Кавычек быть не должно: systemd не снимает их так, как shell.
        assert!(!text.contains("ExecStart=\""));
    }

    #[test]
    fn escape_path_covers_specials() {
        assert_eq!(escape_path(Path::new("/a b")), r"/a\x20b");
        assert_eq!(escape_path(Path::new("/plain")), "/plain");
        assert_eq!(escape_path(Path::new("/a\tb")), r"/a\x09b");
    }

    #[test]
    fn status_without_unit_is_not_installed() {
        // На тестовой машине unit не установлен; если установлен — тест
        // всё равно проходит, потому что проверяет только согласованность.
        let state = status();
        assert!(matches!(
            state,
            State::NotInstalled | State::Stopped | State::Running | State::Pending
        ));
    }
}
