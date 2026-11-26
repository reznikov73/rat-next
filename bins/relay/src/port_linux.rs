//! Поиск процессов, занимающих TCP-порт, и их завершение — Linux.
//!
//! Заменяет `port.rs` с `netstat -ano -p tcp` / `tasklist` / `taskkill`.
//! Здесь: `ss` (iproute2) как основной источник, `lsof` как резервный.
//! Завершение — сигналом через `kill`, а не `taskkill /F`.

use std::path::Path;
use std::process::Command;

/// Процесс, слушающий порт.
#[derive(Debug, Clone)]
pub struct Owner {
    pub pid: u32,
    pub name: String,
}

impl Owner {
    pub fn label(&self) -> String {
        format!("{} (PID {})", self.name, self.pid)
    }
}

/// Имя процесса по PID из `/proc/<pid>/comm`.
fn name_of(pid: u32) -> String {
    let path = format!("/proc/{pid}/comm");
    std::fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "?".to_string())
}

/// Разобрать вывод `ss -ltnpH 'sport = :PORT'`.
///
/// Формат строки (iproute2):
/// `LISTEN 0 4096 0.0.0.0:8889 0.0.0.0:* users:(("rat-relay",pid=1234,fd=3))`
fn parse_ss(text: &str) -> Vec<Owner> {
    let mut out: Vec<Owner> = Vec::new();
    for line in text.lines() {
        // Ищем вхождения `pid=NNN` внутри users:(...)
        let Some(pos) = line.find("pid=") else {
            continue;
        };
        let rest = &line[pos + 4..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let Ok(pid) = digits.parse::<u32>() else {
            continue;
        };

        // Имя процесса: users:(("name",pid=...))
        let name = line
            .find("((\"")
            .and_then(|i| {
                let s = &line[i + 3..];
                s.find('"').map(|j| s[..j].to_string())
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| name_of(pid));

        if !out.iter().any(|o| o.pid == pid) {
            out.push(Owner { pid, name });
        }
    }
    out
}

/// Разобрать вывод `lsof -nP -iTCP:PORT -sTCP:LISTEN -Fpc`.
///
/// `-F` даёт машиночитаемый формат: строки `p<pid>` и `c<command>`.
fn parse_lsof(text: &str) -> Vec<Owner> {
    let mut out: Vec<Owner> = Vec::new();
    let mut pid: Option<u32> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            pid = rest.parse::<u32>().ok();
        } else if let Some(name) = line.strip_prefix('c')
            && let Some(pid) = pid.take()
            && !out.iter().any(|o| o.pid == pid)
        {
            out.push(Owner {
                pid,
                name: if name.is_empty() {
                    name_of(pid)
                } else {
                    name.to_string()
                },
            });
        }
    }
    out
}

/// Процессы, слушающие `port`.
pub fn owners(port: u16) -> Vec<Owner> {
    // Основной путь: ss из iproute2.
    let expr = format!("sport = :{port}");
    if let Ok(out) = Command::new("ss").args(["-ltnpH", &expr]).output()
        && out.status.success()
    {
        let found = parse_ss(&String::from_utf8_lossy(&out.stdout));
        if !found.is_empty() {
            return found;
        }
    }

    // Резерв: lsof. Требует прав на чужие процессы, поэтому идёт вторым.
    if let Ok(out) = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-Fpc"])
        .output()
    {
        return parse_lsof(&String::from_utf8_lossy(&out.stdout));
    }

    Vec::new()
}

/// Завершить процесс сигналом SIGTERM.
///
/// Своевременно завершающийся процесс не требует `SIGKILL`; если он не
/// ответил за отведённое время, шлём `SIGKILL`.
pub fn kill(pid: u32) -> Result<(), String> {
    if pid == std::process::id() {
        return Err("это процесс самого relay".to_string());
    }
    if pid <= 1 {
        return Err(format!("отказ: PID {pid} — системный процесс"));
    }

    let term = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .output()
        .map_err(|e| format!("kill: {e}"))?;

    if term.status.success() {
        // Даём процессу шанс закрыться сам.
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if !Path::new(&format!("/proc/{pid}")).exists() {
                return Ok(());
            }
        }
    }

    let kill_out = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .output()
        .map_err(|e| format!("kill -KILL: {e}"))?;

    if kill_out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&kill_out.stderr).trim().to_string();
        Err(format!("не удалось завершить PID {pid}: {err}"))
    }
}

/// Проверка существования пути без отдельного импорта — не нужна,
/// используется `std::path::Path` напрямую.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ss_output() {
        let text = "LISTEN 0 4096 0.0.0.0:8889 0.0.0.0:* users:((\"rat-relay\",pid=4242,fd=3))\n\
                    LISTEN 0 4096 [::]:8889 [::]:* users:((\"rat-relay\",pid=4242,fd=4))\n";
        let owners = parse_ss(text);
        assert_eq!(owners.len(), 1, "дубли по PID должны схлопываться");
        assert_eq!(owners[0].pid, 4242);
        assert_eq!(owners[0].name, "rat-relay");
        assert_eq!(owners[0].label(), "rat-relay (PID 4242)");
    }

    #[test]
    fn parses_ss_without_users() {
        // Порт занят, но владелец не виден (нет прав) — пустой список,
        // а не паника.
        assert!(parse_ss("LISTEN 0 4096 0.0.0.0:8889 0.0.0.0:*\n").is_empty());
    }

    #[test]
    fn parses_lsof_output() {
        let text = "p1234\ncnginx\np5678\ncnode\n";
        let owners = parse_lsof(text);
        assert_eq!(owners.len(), 2);
        assert_eq!(owners[0].pid, 1234);
        assert_eq!(owners[0].name, "nginx");
        assert_eq!(owners[1].name, "node");
    }

    #[test]
    fn refuses_to_kill_self() {
        let me = std::process::id();
        assert!(kill(me).is_err());
    }

    #[test]
    fn refuses_to_kill_init() {
        assert!(kill(1).is_err());
    }
}
