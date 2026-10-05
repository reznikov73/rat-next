//! Поиск процессов, занимающих TCP-порт, и их завершение (Windows).

use std::process::Command;

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

/// Процессы, слушающие `port` (TCP, IPv4 и IPv6).
pub fn owners(port: u16) -> Vec<Owner> {
    let Ok(out) = hidden("netstat").args(["-ano", "-p", "tcp"]).output() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut pids: Vec<u32> = Vec::new();
    for line in text.lines() {
        // Формат: TCP  <local>  <foreign>  <state>  <pid>
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 5 || !cols[0].eq_ignore_ascii_case("TCP") {
            continue;
        }
        if port_suffix(cols[1]) != Some(port) {
            continue;
        }
        // Пустой удалённый адрес — признак слушающего сокета (не зависит от локали).
        if !is_null_endpoint(cols[2]) {
            continue;
        }
        if let Ok(pid) = cols[4].parse::<u32>()
            && pid != 0
            && !pids.contains(&pid)
        {
            pids.push(pid);
        }
    }
    pids.into_iter()
        .map(|pid| Owner {
            pid,
            name: name_of(pid),
        })
        .collect()
}

/// Команда без мелькания консольного окна.
fn hidden(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Завершить процесс. Свой собственный PID не трогаем.
pub fn kill(pid: u32) -> Result<(), String> {
    if pid == std::process::id() {
        return Err("это процесс самого relay".to_string());
    }
    let out = hidden("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output()
        .map_err(|e| format!("taskkill: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("не удалось завершить PID {pid}")
        } else {
            err
        })
    }
}

fn name_of(pid: u32) -> String {
    let out = hidden("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output();
    let Ok(out) = out else {
        return "неизвестный процесс".to_string();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next().unwrap_or("");
    match first.split(',').next() {
        Some(name) if !name.trim().is_empty() => name.trim().trim_matches('"').to_string(),
        _ => "неизвестный процесс".to_string(),
    }
}

fn port_suffix(endpoint: &str) -> Option<u16> {
    endpoint.rsplit_once(':').and_then(|(_, p)| p.parse().ok())
}

fn is_null_endpoint(endpoint: &str) -> bool {
    matches!(endpoint, "0.0.0.0:0" | "[::]:0" | "*:*")
}
