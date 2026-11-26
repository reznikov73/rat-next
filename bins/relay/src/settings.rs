//! Настройки relay, которые переживают перезапуск (`relay.json` рядом с exe).

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Публичный адрес relay, который вводят клиенты Rat Next.
pub const DEFAULT_PUBLIC: &str = "194.37.254.233:8889";
/// Адрес, на котором relay принимает подключения.
pub const DEFAULT_LISTEN: &str = "0.0.0.0:8889";
/// Порт relay по умолчанию.
pub const DEFAULT_PORT: u16 = 8889;

fn default_public() -> String {
    DEFAULT_PUBLIC.to_string()
}

fn default_listen() -> String {
    DEFAULT_LISTEN.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Адрес для клиентов (внешний адрес relay) — информационное поле.
    #[serde(default = "default_public")]
    pub public_addr: String,
    /// Локальный адрес прослушивания.
    #[serde(default = "default_listen")]
    pub listen_addr: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            public_addr: default_public(),
            listen_addr: default_listen(),
        }
    }
}

/// Файл настроек.
///
/// Windows: рядом с исполняемым файлом (поведение не меняем).
/// Linux: сначала `/etc/rat-next/relay.json` (штатное место для сервиса,
/// который работает не от того пользователя, который может писать рядом с
/// exe), затем `$XDG_CONFIG_HOME/rat-next/relay.json` или
/// `~/.config/rat-next/relay.json`, и только потом — рядом с exe.
///
/// Если файла нет или он нечитаем, [`load`] отдаёт значения по умолчанию.
pub fn path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("relay.json")))
    }

    #[cfg(not(windows))]
    {
        // 1. Системный конфиг — основной для сервиса.
        let system = PathBuf::from("/etc/rat-next/relay.json");
        if system.exists() {
            return Some(system);
        }

        // 2. Пользовательский конфиг.
        let user = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
            _ => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")),
        }
        .map(|base| base.join("rat-next").join("relay.json"));
        if let Some(p) = &user
            && p.exists()
        {
            return Some(p.clone());
        }

        // 3. Рядом с exe — как на Windows (переносимый запуск из папки).
        if let Some(p) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("relay.json")))
            .filter(|p| p.exists())
        {
            return Some(p);
        }

        // Файла нигде нет — вернём предпочтительное место для записи.
        user.or(Some(system))
    }
}

/// Читает настройки; при отсутствии или ошибке возвращает значения по умолчанию.
pub fn load() -> Settings {
    let Some(p) = path() else {
        return Settings::default();
    };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return Settings::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save(settings: &Settings) -> Result<(), String> {
    let Some(p) = path() else {
        return Err("не удалось определить папку рядом с exe".to_string());
    };
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))
}

/// Дописывает порт по умолчанию, если он не указан.
pub fn normalize(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return String::new();
    }
    if s.parse::<SocketAddr>().is_ok() {
        return s.to_string();
    }
    // IPv6 без порта: `[::1]` → `[::1]:8889`.
    if s.starts_with('[') && !s.contains("]:") {
        return format!("{s}:{DEFAULT_PORT}");
    }
    if !s.contains(':') {
        return format!("{s}:{DEFAULT_PORT}");
    }
    s.to_string()
}

/// Адрес пригоден для `TcpListener::bind`.
pub fn validate_listen(input: &str) -> Result<String, String> {
    let addr = normalize(input);
    if addr.is_empty() {
        return Err("укажите адрес прослушивания".to_string());
    }
    addr.parse::<SocketAddr>()
        .map(|a| a.to_string())
        .map_err(|_| format!("некорректный адрес: {addr} (пример: {DEFAULT_LISTEN})"))
}

/// Адрес для клиентов: допускаем имя хоста, но порт обязателен.
pub fn validate_public(input: &str) -> Result<String, String> {
    let addr = normalize(input);
    if addr.is_empty() {
        return Err("укажите адрес для клиентов".to_string());
    }
    match addr.parse::<SocketAddr>() {
        Ok(a) => Ok(a.to_string()),
        Err(_) => {
            let Some((host, port)) = addr.rsplit_once(':') else {
                return Err(format!("не указан порт (пример: {DEFAULT_PUBLIC})"));
            };
            port.parse::<u16>()
                .map_err(|_| format!("некорректный порт: {port}"))?;
            if host.is_empty() {
                return Err("не указан адрес".to_string());
            }
            Ok(format!("{host}:{port}"))
        }
    }
}

/// Порт из адреса прослушивания (для поиска занявшего процесса).
pub fn port_of(addr: &str) -> Option<u16> {
    addr.parse::<SocketAddr>().ok().map(|a| a.port())
}
