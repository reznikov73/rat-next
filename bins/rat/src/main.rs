//! `rat` — единое приложение Rat Next: морда с плитками, встроенный сервер
//! (приём подключений) и вьюер в одном окне.
//!
//! Пилот: машины адресуются как `IP:порт` (локальная сеть / Radmin VPN).
//! Позже (P4) адрес заменится на ID через relay/signaling — морда не изменится.
//!
//! Запуск:
//! ```text
//! rat.exe                                  # сервер стартует автоматически, порт 55140
//! rat.exe --port 55140
//! rat.exe --monitor 1                      # отдавать второй экран (индекс 1)
//! rat.exe --switch-after=4                 # тест: через 4 сек переключить монитор
//! rat.exe --autoconnect 192.168.1.50:55140
//! rat.exe --relay 91.x.x.x:55141 --device-id 123456           # интернет-режим
//! rat.exe --tray                              # стартовать сразу в трее (без окна)
//! rat.exe --install-service                   # служба: доступен с загрузки Windows (рекомендуется)
//! rat.exe --uninstall-service                 # снять службу
//! rat.exe --install-autostart                 # автозапуск при входе в Windows
//! rat.exe --remove-autostart                  # выключить автозапуск
//! rat.exe --smoke=15                       # авто-выход через 15 сек (тесты)
//! ```
#![windows_subsystem = "windows"]

#[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
struct Args {
    port: u16,
    monitor: u32,
    autoconnect: Option<String>,
    smoke: Option<f32>,
    switch_after: Option<f32>,
    relay: Option<String>,
    device_id: Option<String>,
    tray: bool,
    install_autostart: bool,
    remove_autostart: bool,
    install_service: bool,
    uninstall_service: bool,
    start_service: bool,
    stop_service: bool,
    service: bool,
    service_helper: bool,
    probe_secure: bool,
}

#[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
fn parse_args() -> Args {
    let mut a = Args {
        port: 55140,
        monitor: 0,
        autoconnect: None,
        smoke: None,
        switch_after: None,
        relay: None,
        device_id: None,
        tray: false,
        install_autostart: false,
        remove_autostart: false,
        install_service: false,
        uninstall_service: false,
        start_service: false,
        stop_service: false,
        service: false,
        service_helper: false,
        probe_secure: false,
    };
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        let cur = args[i].as_str();
        match cur {
            "--port" if i + 1 < args.len() => {
                a.port = args[i + 1].parse().unwrap_or(55140);
                i += 2;
            }
            "--monitor" if i + 1 < args.len() => {
                a.monitor = args[i + 1].parse().unwrap_or(0);
                i += 2;
            }
            "--autoconnect" if i + 1 < args.len() => {
                a.autoconnect = Some(args[i + 1].clone());
                i += 2;
            }
            "--install-autostart" => {
                a.install_autostart = true;
                i += 1;
            }
            "--remove-autostart" => {
                a.remove_autostart = true;
                i += 1;
            }
            "--tray" => {
                a.tray = true;
                i += 1;
            }
            "--install-service" => {
                a.install_service = true;
                i += 1;
            }
            "--uninstall-service" => {
                a.uninstall_service = true;
                i += 1;
            }
            "--start-service" => {
                a.start_service = true;
                i += 1;
            }
            "--stop-service" => {
                a.stop_service = true;
                i += 1;
            }
            "--service" => {
                a.service = true;
                i += 1;
            }
            "--probe-secure" => {
                a.probe_secure = true;
                i += 1;
            }
            "--service-helper" => {
                a.service_helper = true;
                i += 1;
            }
            "--relay" if i + 1 < args.len() => {
                a.relay = Some(args[i + 1].clone());
                i += 2;
            }
            "--device-id" if i + 1 < args.len() => {
                a.device_id = Some(args[i + 1].clone());
                i += 2;
            }
            s if s.starts_with("--switch-after=") => {
                a.switch_after = s["--switch-after=".len()..].parse().ok();
                i += 1;
            }
            s if s.starts_with("--smoke=") => {
                a.smoke = s["--smoke=".len()..].parse().ok();
                i += 1;
            }
            other => {
                eprintln!("неизвестный аргумент: {other}");
                i += 1;
            }
        }
    }
    a
}

#[cfg(not(feature = "ffmpeg"))]
fn main() {
    let _ = parse_args();
    eprintln!("rat: соберите с --features ffmpeg (пилотный кодек-путь)");
    std::process::exit(3);
}

#[cfg(feature = "ffmpeg")]
fn main() {
    let args = parse_args();
    if args.install_service {
        match service::install() {
            Ok(()) => {
                note("service: установлена служба «RatNext» (задача автозапуска убрана)");
                println!("Служба установлена: агент будет доступен с момента загрузки Windows.");
                std::process::exit(0);
            }
            Err(e) => {
                note(&format!("service: ошибка установки: {e}"));
                eprintln!("service: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.uninstall_service {
        match service::uninstall() {
            Ok(()) => {
                note("service: служба удалена");
                println!("Служба удалена.");
                std::process::exit(0);
            }
            Err(e) => {
                note(&format!("service: ошибка удаления: {e}"));
                eprintln!("service: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.start_service {
        match service::start() {
            Ok(()) => {
                note("service: служба запущена (из интерфейса)");
                println!("Служба запущена.");
                std::process::exit(0);
            }
            Err(e) => {
                note(&format!("service: ошибка запуска: {e}"));
                eprintln!("service: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.stop_service {
        match service::stop() {
            Ok(()) => {
                note("service: служба остановлена (из интерфейса)");
                println!("Служба остановлена.");
                std::process::exit(0);
            }
            Err(e) => {
                note(&format!("service: ошибка остановки: {e}"));
                eprintln!("service: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.probe_secure {
        // Диагностика защищённого стола: имя активного стола + пробный снимок.
        let name = rat_session::secure::active_desktop_name();
        note(&format!("probe-secure: активный стол = {name:?}"));
        match rat_session::secure::SecureCapture::attach() {
            Ok(mut sc) => {
                let (w, h) = sc.size();
                let mut frame: Vec<u8> = Vec::new();
                let res = sc.capture(|bgra, _pitch| {
                    frame = bgra.to_vec();
                });
                note(&format!("probe-secure: захват {w}x{h}: {res:?}"));
                if matches!(res, Ok(true))
                    && let Ok(exe) = std::env::current_exe()
                {
                    let bmp = exe.with_file_name("probe-secure.bmp");
                    write_bmp(&bmp, w, h, &frame);
                    note(&format!("probe-secure: снимок сохранён: {}", bmp.display()));
                }
            }
            Err(e) => note(&format!("probe-secure: ошибка: {e}")),
        }
        std::process::exit(0);
    }
    if args.service {
        if let Err(e) = service::run_service() {
            note(&format!("service: {e}"));
            std::process::exit(1);
        }
        return;
    }
    if args.install_autostart {
        match autostart::install() {
            Ok(()) => {
                println!("Автозапуск включён (задача «Rat Next»).");
                note("autostart: установлен");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("autostart: {e}");
                note(&format!("autostart: ошибка установки: {e}"));
                std::process::exit(1);
            }
        }
    }
    if args.remove_autostart {
        match autostart::remove() {
            Ok(()) => {
                println!("Автозапуск выключен.");
                note("autostart: удалён");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("autostart: {e}");
                note(&format!("autostart: ошибка удаления: {e}"));
                std::process::exit(1);
            }
        }
    }
    if !rat_session::single_instance::try_acquire("app") {
        if args.smoke.is_none() && !args.service_helper {
            // Повторный запуск — просьба к работающему экземпляру показать окно
            // (то же, что клик по значку в трее; работает и когда значка нет).
            if rat_session::single_instance::show_signal("app") {
                note("rat: повторный запуск — отправлен запрос «показать окно»");
                std::process::exit(0);
            }
            rat_session::single_instance::warn(
                "Rat Next",
                "Rat Next уже запущен.\nОкно можно открыть из значка в трее.",
            );
        }
        note("rat: экземпляр уже запущен (двойной запуск заблокирован)");
        std::process::exit(2);
    }
    if let Err(e) = ui::run(args) {
        note(&format!("rat: {e}"));
        std::process::exit(1);
    }
}

// ================== журнал ==================

/// Сохранить снимок BGRA (сверху вниз) в BMP-файл (для диагностики).
#[cfg(feature = "ffmpeg")]
fn write_bmp(path: &std::path::Path, w: u32, h: u32, bgra: &[u8]) {
    use std::io::Write as _;
    let row = (w as usize) * 4;
    if bgra.len() < row * (h as usize) {
        return;
    }
    let data_size = row * (h as usize);
    let mut hdr = Vec::with_capacity(54);
    hdr.extend_from_slice(b"BM");
    hdr.extend_from_slice(&((54 + data_size) as u32).to_le_bytes());
    hdr.extend_from_slice(&[0u8; 4]);
    hdr.extend_from_slice(&54u32.to_le_bytes());
    hdr.extend_from_slice(&40u32.to_le_bytes());
    hdr.extend_from_slice(&(w as i32).to_le_bytes());
    hdr.extend_from_slice(&(h as i32).to_le_bytes());
    hdr.extend_from_slice(&1u16.to_le_bytes());
    hdr.extend_from_slice(&32u16.to_le_bytes());
    hdr.extend_from_slice(&0u32.to_le_bytes());
    hdr.extend_from_slice(&(data_size as u32).to_le_bytes());
    hdr.extend_from_slice(&2835u32.to_le_bytes());
    hdr.extend_from_slice(&2835u32.to_le_bytes());
    hdr.extend_from_slice(&0u32.to_le_bytes());
    hdr.extend_from_slice(&0u32.to_le_bytes());
    if let Ok(mut f) = std::fs::File::create(path) {
        let _ = f.write_all(&hdr);
        for y in (0..h as usize).rev() {
            let _ = f.write_all(&bgra[y * row..(y + 1) * row]);
        }
    }
}

/// Записать строку в `rat.log` рядом с exe (переживает отсутствие консоли).
#[cfg(feature = "ffmpeg")]
fn log_line(s: &str) {
    use std::io::Write as _;
    let path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("rat.log")));
    if let Some(p) = path
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p)
    {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "{t} {s}");
    }
}

/// Лог: в stdout (если он есть) и в `rat.log`.
#[cfg(feature = "ffmpeg")]
fn note(s: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stdout(), "{s}");
    log_line(s);
}

// ================== значок приложения и трей ==================

/// Рисование иконки: тот же код использует `build.rs`, чтобы вложить `.ico` в exe.
#[cfg(feature = "ffmpeg")]
mod icon;

/// Значок в области уведомлений.
#[cfg(feature = "ffmpeg")]
mod elevate;
#[cfg(feature = "ffmpeg")]
mod service;

#[cfg(feature = "ffmpeg")]
mod tray;

// ================== хранилище машин ==================

#[cfg(feature = "ffmpeg")]
mod store {
    use serde::{Deserialize, Serialize};
    use std::path::PathBuf;

    #[derive(Serialize, Deserialize, Clone)]
    pub struct Machine {
        pub name: String,
        pub addr: String,
        /// Unix-время последнего подключения.
        pub last_connected: Option<i64>,
    }

    pub fn path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("machines.json")))
            .unwrap_or_else(|| PathBuf::from("machines.json"))
    }

    pub fn load() -> Vec<Machine> {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(list: &[Machine]) {
        if let Ok(s) = serde_json::to_string_pretty(list) {
            let _ = std::fs::write(path(), s);
        }
    }

    pub const DEFAULT_RELAY: &str = "194.37.254.233:8889";

    #[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
    #[serde(rename_all = "snake_case")]
    pub enum ConnectionMode {
        #[default]
        Internet,
        Lan,
    }

    /// Настройки этого узла (device.json рядом с exe).
    #[derive(Serialize, Deserialize, Clone)]
    #[serde(default)]
    pub struct Device {
        pub id: String,
        pub relay: String,
        pub mode: ConnectionMode,
        /// Принимать входящие подключения без запроса подтверждения.
        pub auto_allow: bool,
        /// Прятать окно в трей вместо выхода по крестику.
        pub tray: bool,
    }

    impl Default for Device {
        fn default() -> Self {
            Self {
                id: String::new(),
                relay: DEFAULT_RELAY.to_string(),
                mode: ConnectionMode::Internet,
                auto_allow: false,
                tray: true,
            }
        }
    }

    pub fn device_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("device.json")))
            .unwrap_or_else(|| PathBuf::from("device.json"))
    }

    pub fn load_device() -> Device {
        let mut device = std::fs::read_to_string(device_path())
            .ok()
            .and_then(|s| serde_json::from_str::<Device>(&s).ok())
            .unwrap_or_default();
        device.relay = normalize_relay(&device.relay);
        device
    }

    pub fn save_device(d: &Device) {
        if let Ok(s) = serde_json::to_string_pretty(d) {
            let _ = std::fs::write(device_path(), s);
        }
    }

    pub fn is_valid_id(id: &str) -> bool {
        id.len() == 6 && id.bytes().all(|byte| byte.is_ascii_digit())
    }

    pub fn gen_id() -> Result<String, std::io::Error> {
        let mut bytes = [0_u8; 4];
        fill_random(&mut bytes)?;
        let number = 100_000 + u32::from_le_bytes(bytes) % 900_000;
        Ok(format!("{number:06}"))
    }

    #[cfg(windows)]
    fn fill_random(bytes: &mut [u8]) -> Result<(), std::io::Error> {
        #[link(name = "bcrypt")]
        unsafe extern "system" {
            fn BCryptGenRandom(
                algorithm: *mut std::ffi::c_void,
                buffer: *mut u8,
                length: u32,
                flags: u32,
            ) -> i32;
        }
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                2,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(std::io::Error::other(format!("BCryptGenRandom: {status}")))
        }
    }

    #[cfg(not(windows))]
    fn fill_random(bytes: &mut [u8]) -> Result<(), std::io::Error> {
        use std::io::Read;
        std::fs::File::open("/dev/urandom")?.read_exact(bytes)
    }

    pub fn normalize_relay(raw: &str) -> String {
        let relay = raw.trim();
        if relay.is_empty() {
            return DEFAULT_RELAY.to_string();
        }
        let has_port = relay
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse::<u16>().ok())
            .is_some();
        if has_port || relay.contains(':') {
            relay.to_string()
        } else {
            format!("{relay}:8889")
        }
    }

    /// Нормализовать: IP → добавить порт по умолчанию; ID — как есть.
    pub fn normalize_addr(raw: &str) -> String {
        let s = raw.trim();
        if s.contains(':') {
            s.to_string()
        } else if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.') {
            format!("{s}:55140")
        } else {
            s.to_string()
        }
    }

    pub fn now_unix() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    pub fn ago_text(ts: i64) -> String {
        let d = now_unix() - ts;
        if d < 60 {
            "только что".into()
        } else if d < 3600 {
            format!("{} мин назад", d / 60)
        } else if d < 86400 {
            format!("{} ч назад", d / 3600)
        } else {
            format!("{} дн назад", d / 86400)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn generated_id_has_six_digits() {
            let id = gen_id().unwrap();
            assert!(is_valid_id(&id));
        }

        #[test]
        fn relay_address_gets_default_port() {
            assert_eq!(normalize_relay("194.37.254.233"), "194.37.254.233:8889");
            assert_eq!(normalize_relay(""), DEFAULT_RELAY);
            assert_eq!(normalize_relay("relay.example:9000"), "relay.example:9000");
            assert_eq!(normalize_relay("relay.example:bad"), "relay.example:bad");
        }
    }
}

#[cfg(feature = "ffmpeg")]
mod notify {
    //! Системные сигналы (звук) — без внешних зависимостей.

    /// Короткий системный сигнал (запрос подключения).
    pub fn beep() {
        #[cfg(windows)]
        unsafe {
            MessageBeep(0x0000_0040); // MB_ICONASTERISK
        }
    }

    #[cfg(windows)]
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBeep(u_type: u32) -> i32;
    }
}

#[cfg(feature = "ffmpeg")]
mod autostart {
    //! Автозапуск при входе пользователя через Планировщик задач (`schtasks`).

    const TASK: &str = "Rat Next";

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
    pub fn is_installed() -> bool {
        schtasks()
            .args(["/Query", "/TN", TASK])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Создать (или пересоздать) задачу: запуск при входе, сразу в трей.
    pub fn install() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let tr = format!("\"{}\" --tray", exe.display());
        let out = schtasks()
            .args([
                // HIGHEST: агент должен работать с повышенными правами, иначе Windows
                // (UIPI) не пропускает ввод в окна администраторских процессов —
                // «мышь работает только в самом окне rat».
                "/Create", "/F", "/SC", "ONLOGON", "/TN", TASK, "/TR", &tr, "/RL", "HIGHEST",
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
}

#[cfg(feature = "ffmpeg")]
mod approval {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Sender, SyncSender};
    use std::time::{Duration, Instant};

    #[derive(Clone, Copy)]
    pub enum Source {
        Lan,
        Internet,
    }

    pub struct Request {
        pub source: Source,
        pub expires_at: Instant,
        pub reply: SyncSender<bool>,
    }

    /// Настройка «Подключать без подтверждения» из окна настроек.
    static AUTO_ALLOW: AtomicBool = AtomicBool::new(false);

    pub fn set_auto_allow(on: bool) {
        AUTO_ALLOW.store(on, Ordering::Relaxed);
    }

    pub fn request(requests: &Sender<Request>, source: Source, running: &AtomicBool) -> bool {
        if AUTO_ALLOW.load(Ordering::Relaxed) {
            crate::note(&format!(
                "[guard] подключение разрешено без подтверждения ({})",
                match source {
                    Source::Lan => "локальная сеть",
                    Source::Internet => "интернет",
                }
            ));
            return true;
        }
        let (reply, decision) = mpsc::sync_channel(1);
        let expires_at = Instant::now() + Duration::from_secs(60);
        if requests
            .send(Request {
                source,
                expires_at,
                reply,
            })
            .is_err()
        {
            return false;
        }
        loop {
            if !running.load(Ordering::Relaxed) {
                return false;
            }
            let remaining = expires_at.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            match decision.recv_timeout(remaining.min(Duration::from_millis(200))) {
                Ok(allowed) => return allowed,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return false,
            }
        }
    }
}

// ================== встроенный сервер ==================

#[cfg(feature = "ffmpeg")]
mod server {
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use std::sync::mpsc::Sender;

    use rat_session::agent::{AgentConfig, spawn_agent};
    use rat_session::link::tcp_accept;
    use rat_session::stats::{SessionStats, TimeBase};

    pub struct ServerHandle {
        pub port: u16,
        running: Arc<AtomicBool>,
        stop_current: Arc<Mutex<Option<Arc<AtomicBool>>>>,
        thread: Option<JoinHandle<()>>,
        incoming_requests: Sender<crate::approval::Request>,
        pub sessions: Arc<AtomicU64>,
        /// Какой монитор отдавать (индекс DXGI; применяется к новым сессиям).
        pub monitor: Arc<AtomicU32>,
        pub last_err: Arc<Mutex<Option<String>>>,
    }

    impl ServerHandle {
        pub fn new(incoming_requests: Sender<crate::approval::Request>) -> Self {
            Self {
                port: 55140,
                running: Arc::new(AtomicBool::new(false)),
                stop_current: Arc::new(Mutex::new(None)),
                thread: None,
                incoming_requests,
                sessions: Arc::new(AtomicU64::new(0)),
                monitor: Arc::new(AtomicU32::new(0)),
                last_err: Arc::new(Mutex::new(None)),
            }
        }

        pub fn is_running(&self) -> bool {
            self.running.load(Ordering::Relaxed)
        }

        pub fn start(&mut self, port: u16) -> Result<(), String> {
            if self.is_running() {
                return Ok(());
            }
            let listener = TcpListener::bind(("0.0.0.0", port))
                .map_err(|e| format!("не удалось занять порт {port}: {e}"))?;
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
            *self.last_err.lock().unwrap() = None;
            self.running.store(true, Ordering::Relaxed);
            self.port = port;
            let running = self.running.clone();
            let stop_current = self.stop_current.clone();
            let incoming_requests = self.incoming_requests.clone();
            let sessions = self.sessions.clone();
            let monitor = self.monitor.clone();
            let last_err = self.last_err.clone();
            crate::note(&format!("[server] слушаю 0.0.0.0:{port}"));
            self.thread = Some(std::thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    match tcp_accept(&listener) {
                        Ok((link, closer)) => {
                            let approved = crate::approval::request(
                                &incoming_requests,
                                crate::approval::Source::Lan,
                                &running,
                            );
                            if !approved {
                                closer.close();
                                continue;
                            }
                            crate::note("[server] клиент подключился");
                            sessions.fetch_add(1, Ordering::Relaxed);
                            let base = TimeBase(Instant::now());
                            let stop = Arc::new(AtomicBool::new(false));
                            *stop_current.lock().unwrap() = Some(stop.clone());
                            let stats = Arc::new(Mutex::new(SessionStats::default()));
                            let cfg = AgentConfig {
                                monitor: monitor.load(Ordering::Relaxed),
                                ..AgentConfig::default()
                            };
                            match spawn_agent(base, stop.clone(), stats.clone(), link, cfg) {
                                Ok(handles) => {
                                    for h in handles {
                                        let _ = h.join();
                                    }
                                    closer.close();
                                    let st = stats.lock().unwrap();
                                    crate::note(&format!(
                                        "[server] сессия завершена\n{}",
                                        st.summary()
                                    ));
                                }
                                Err(e) => {
                                    closer.close();
                                    crate::note(&format!("[server] ошибка сессии: {e}"));
                                    *last_err.lock().unwrap() = Some(e);
                                }
                            }
                            *stop_current.lock().unwrap() = None;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(150));
                        }
                        Err(e) => {
                            *last_err.lock().unwrap() = Some(format!("accept: {e}"));
                            std::thread::sleep(Duration::from_millis(500));
                        }
                    }
                }
                crate::note("[server] остановлен");
            }));
            Ok(())
        }

        pub fn stop(&mut self) {
            self.running.store(false, Ordering::Relaxed);
            if let Some(s) = self.stop_current.lock().unwrap().take() {
                s.store(true, Ordering::Relaxed);
            }
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
}

// ================== интернет-режим (relay-клиент) ==================

#[cfg(feature = "ffmpeg")]
mod internet {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use rat_session::agent::{AgentConfig, spawn_agent};
    use rat_session::link::{TcpCloser, relay_register, relay_wait_paired};
    use rat_session::stats::{SessionStats, TimeBase};

    pub struct InternetHandle {
        running: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
        incoming_requests: Sender<crate::approval::Request>,
        pub status: Arc<Mutex<String>>,
        pub sessions: Arc<AtomicU64>,
        abort: Arc<Mutex<Option<TcpCloser>>>,
        stop_current: Arc<Mutex<Option<Arc<AtomicBool>>>>,
        monitor: Arc<std::sync::atomic::AtomicU32>,
    }

    impl InternetHandle {
        pub fn new(
            monitor: Arc<std::sync::atomic::AtomicU32>,
            incoming_requests: Sender<crate::approval::Request>,
        ) -> Self {
            Self {
                running: Arc::new(AtomicBool::new(false)),
                thread: None,
                incoming_requests,
                status: Arc::new(Mutex::new("выключен".to_string())),
                sessions: Arc::new(AtomicU64::new(0)),
                abort: Arc::new(Mutex::new(None)),
                stop_current: Arc::new(Mutex::new(None)),
                monitor,
            }
        }

        pub fn is_running(&self) -> bool {
            self.running.load(Ordering::Relaxed)
        }

        pub fn start(&mut self, relay: String, id: String, name: String) {
            if self.is_running() {
                return;
            }
            self.running.store(true, Ordering::Relaxed);
            let running = self.running.clone();
            let status = self.status.clone();
            let sessions = self.sessions.clone();
            let abort = self.abort.clone();
            let stop_current = self.stop_current.clone();
            let incoming_requests = self.incoming_requests.clone();
            let monitor = self.monitor.clone();
            self.thread = Some(std::thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    crate::note(&format!("[internet] подключаюсь: {relay} ({id})"));
                    *status.lock().unwrap() = "подключаюсь к серверу…".to_string();
                    let reg = relay_register(relay.as_str(), &id, &name);
                    let (stream, aborter) = match reg {
                        Ok(v) => v,
                        Err(e) => {
                            crate::note(&format!("[internet] ошибка подключения: {e}"));
                            *status.lock().unwrap() = format!("ошибка: {e}");
                            sleep_checked(&running, 5000);
                            continue;
                        }
                    };
                    crate::note(&format!("[internet] зарегистрирован: {id}"));
                    *abort.lock().unwrap() = Some(aborter);
                    *status.lock().unwrap() =
                        "ожидаю подключения (сообщите ID оператору)".to_string();
                    match relay_wait_paired(stream) {
                        Err(e) => {
                            *abort.lock().unwrap() = None;
                            if running.load(Ordering::Relaxed) {
                                crate::note(&format!("[internet] соединение закрыто: {e}"));
                                *status.lock().unwrap() = format!("соединение закрыто: {e}");
                                sleep_checked(&running, 3000);
                            }
                            continue;
                        }
                        Ok((link, closer)) => {
                            *abort.lock().unwrap() = None;
                            *status.lock().unwrap() =
                                "ожидаю разрешения на подключение".to_string();
                            let approved = crate::approval::request(
                                &incoming_requests,
                                crate::approval::Source::Internet,
                                &running,
                            );
                            if !approved {
                                closer.close();
                                *status.lock().unwrap() =
                                    "подключение отклонено или истёк запрос".to_string();
                                continue;
                            }
                            *status.lock().unwrap() = "сессия (через сервер)".to_string();
                            sessions.fetch_add(1, Ordering::Relaxed);
                            let mon = monitor.load(Ordering::Relaxed);
                            serve_session(link, closer, &stop_current, mon);
                        }
                    }
                }
                *status.lock().unwrap() = "выключен".to_string();
            }));
        }

        pub fn stop(&mut self) {
            self.running.store(false, Ordering::Relaxed);
            if let Some(a) = self.abort.lock().unwrap().take() {
                a.close();
            }
            if let Some(s) = self.stop_current.lock().unwrap().take() {
                s.store(true, Ordering::Relaxed);
            }
            if let Some(t) = self.thread.take() {
                std::thread::spawn(move || {
                    let _ = t.join();
                });
            }
        }
    }

    fn sleep_checked(running: &AtomicBool, ms: u64) {
        let mut left = ms;
        while left > 0 && running.load(Ordering::Relaxed) {
            let step = left.min(250);
            std::thread::sleep(Duration::from_millis(step));
            left -= step;
        }
    }

    /// Обслужить одну сессию поверх готового линка (relay-путь).
    fn serve_session(
        link: rat_session::link::Duplex,
        closer: TcpCloser,
        stop_current: &Arc<Mutex<Option<Arc<AtomicBool>>>>,
        monitor: u32,
    ) {
        let base = TimeBase(Instant::now());
        let stop = Arc::new(AtomicBool::new(false));
        *stop_current.lock().unwrap() = Some(stop.clone());
        let stats = Arc::new(Mutex::new(SessionStats::default()));
        let cfg = AgentConfig {
            monitor,
            ..AgentConfig::default()
        };
        match spawn_agent(base, stop.clone(), stats.clone(), link, cfg) {
            Ok(handles) => {
                for h in handles {
                    let _ = h.join();
                }
                closer.close();
                let st = stats.lock().unwrap();
                crate::note(&format!("[internet] сессия завершена\n{}", st.summary()));
            }
            Err(e) => {
                closer.close();
                crate::note(&format!("[internet] ошибка сессии: {e}"));
            }
        }
        *stop_current.lock().unwrap() = None;
    }
}

// ================== приложение (egui) ==================

#[cfg(feature = "ffmpeg")]
mod ui {
    use std::collections::HashMap;
    use std::net::ToSocketAddrs;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, TryRecvError, channel, sync_channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use eframe::egui;
    use rat_session::capture::MonitorInfo;
    use rat_session::input::{self, CursorState, InputEvent, SessionInfo};
    use rat_session::link::{TcpCloser, tcp_connect};
    use rat_session::stats::{SessionStats, TimeBase};
    use rat_session::viewer::{FrameOut, ViewerShared, spawn_viewer};

    use super::Args;
    use super::approval::{Request, Source};
    use super::internet::InternetHandle;
    use super::server::ServerHandle;
    use super::store::{
        ConnectionMode, Device, Machine, ago_text, gen_id, is_valid_id, load, load_device,
        normalize_addr, normalize_relay, now_unix, save, save_device,
    };

    const CARD_BG: egui::Color32 = egui::Color32::from_rgb(34, 39, 47);
    const CARD_BORDER: egui::Color32 = egui::Color32::from_rgb(70, 76, 88);
    const CARD_BORDER_HOVER: egui::Color32 = egui::Color32::from_rgb(64, 140, 255);
    const ON: egui::Color32 = egui::Color32::from_rgb(76, 175, 80);
    const OFF: egui::Color32 = egui::Color32::from_rgb(244, 67, 54);
    const DIM: egui::Color32 = egui::Color32::from_rgb(150, 158, 170);
    const DIM2: egui::Color32 = egui::Color32::from_rgb(120, 128, 140);
    /// Свой ID — самое важное число в главном окне: его диктуют оператору.
    const ID_COLOR: egui::Color32 = egui::Color32::from_rgb(96, 176, 255);
    /// Ещё ярче — для самого номера ID.
    const ID_BRIGHT: egui::Color32 = egui::Color32::from_rgb(140, 205, 255);
    /// Имя семейства с настоящим жирным шрифтом для номера ID.
    const ID_BOLD: &str = "id-bold";

    /// Подключает системный жирный шрифт для номера ID (`strong()` в egui
    /// меняет только цвет — реальной жирности в комплекте нет).
    fn install_ui_fonts(ctx: &egui::Context) {
        let candidates = [
            r"C:\Windows\Fonts\segoeuib.ttf",
            r"C:\Windows\Fonts\seguisb.ttf",
            r"C:\Windows\Fonts\arialbd.ttf",
        ];
        let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else {
            return;
        };
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            ID_BOLD.to_owned(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        let mut family = vec![ID_BOLD.to_owned()];
        // Запасной вариант — шрифт по умолчанию.
        if let Some(default_font) = fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .and_then(|f| f.first().cloned())
        {
            family.push(default_font);
        }
        fonts
            .families
            .insert(egui::FontFamily::Name(ID_BOLD.into()), family);
        ctx.set_fonts(fonts);
    }

    pub fn run(args: Args) -> Result<(), String> {
        if std::env::var_os("WGPU_BACKEND").is_none() {
            // SAFETY: до инициализации GUI, однопоточно.
            unsafe { std::env::set_var("WGPU_BACKEND", "dx12") };
        }
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1020.0, 700.0])
                .with_icon(window_icon()),
            ..Default::default()
        };
        // Слушатель повторного запуска: второй rat.exe просит показать окно.
        let repaint_slot: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let show_request = {
            let slot = repaint_slot.clone();
            rat_session::single_instance::show_listener("app", move || {
                if let Some(ctx) = slot.lock().ok().and_then(|c| c.clone()) {
                    ctx.request_repaint();
                }
            })
        };
        eframe::run_native(
            "Rat Next",
            options,
            Box::new(move |cc| {
                install_ui_fonts(&cc.egui_ctx);
                Ok(Box::new(RatApp::new(args, show_request, repaint_slot)))
            }),
        )
        .map_err(|e| e.to_string())
    }

    /// Иконка окна и панели задач — тот же рисунок, что у трея и у exe.
    fn window_icon() -> egui::IconData {
        const SIDE: u32 = 64;
        egui::IconData {
            rgba: crate::icon::rgba(SIDE),
            width: SIDE,
            height: SIDE,
        }
    }

    type ConnectMsg = Result<ViewerSession, String>;

    enum Screen {
        Home,
        Viewer,
    }

    struct ViewerSession {
        name: String,
        addr: String,
        rx: Receiver<FrameOut>,
        stats: Arc<Mutex<SessionStats>>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
        closer: TcpCloser,
        base: TimeBase,
        texture: Option<egui::TextureHandle>,
        last: Option<FrameOut>,
        input_q: Arc<Mutex<Vec<InputEvent>>>,
        cursor: Arc<Mutex<CursorState>>,
        info: Arc<Mutex<Option<SessionInfo>>>,
        monitors: Arc<Mutex<Vec<(u16, u16, u16)>>>,
        pending_switch: Arc<Mutex<Option<u16>>>,
        // Панели-оверлеи: автоскрытие и анимация выезда.
        top_open: bool,
        bottom_open: bool,
        top_t: f32,
        bottom_t: f32,
        pinned: bool,
        clip_in: Arc<Mutex<Option<String>>>,
        pending_paste: Arc<Mutex<Option<String>>>,
        last_clip_applied: Option<String>,
        switch_after: Option<f32>,
        switch_done: bool,
        connected_at: Instant,
        last_pointer: Option<(u16, u16)>,
        prev_mods: egui::Modifiers,
        buttons_down: [bool; 5],
        ui_e2e_ms: Vec<f32>,
    }

    impl ViewerSession {
        fn shutdown(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
            self.closer.close();
            let st = self.stats.lock().unwrap();
            crate::note(&format!(
                "=== SESSION SUMMARY ({}) ===\n{}",
                self.addr,
                st.summary()
            ));
            if !self.ui_e2e_ms.is_empty() {
                crate::note(&format!(
                    "ui present e2e: avg {:.2}ms p95 {:.2}ms",
                    SessionStats::avg(&self.ui_e2e_ms),
                    SessionStats::pct(&self.ui_e2e_ms, 0.95)
                ));
            }
        }
    }

    enum TileAction {
        Connect(usize),
        Rename(usize),
        Delete(usize),
    }

    pub struct RatApp {
        args: Args,
        machines: Arc<Mutex<Vec<Machine>>>,
        statuses: Arc<Mutex<HashMap<String, bool>>>,
        server: ServerHandle,
        internet: InternetHandle,
        device: Device,
        settings_open: bool,
        settings_mode: ConnectionMode,
        settings_relay_edit: String,
        settings_auto_allow: bool,
        settings_tray: bool,
        settings_autostart: bool,
        settings_error: Option<String>,
        incoming_rx: Receiver<Request>,
        incoming_pending: Option<Request>,
        monitors: Vec<MonitorInfo>,
        monitor_sel: u32,
        port_edit: String,
        my_ip: String,
        my_name: String,
        screen: Screen,
        viewer: Option<ViewerSession>,
        connect_rx: Option<Receiver<ConnectMsg>>,
        connecting_to: Option<String>,
        toast: Option<(String, Instant)>,
        add_open: bool,
        add_name: String,
        add_addr: String,
        add_err: Option<String>,
        edit_open: Option<usize>,
        edit_name: String,
        autoconnect_done: bool,
        started: Instant,
        finished: bool,
        /// Значок в области уведомлений; есть, только если включена настройка.
        tray: Option<crate::tray::Tray>,
        /// Выбран «Выход» — закрытие окна завершает процесс, а не прячет его.
        quitting: bool,
        /// Окно скрыто в трей — перерисовку можно замедлить.
        hidden: bool,
        /// Первый кадр: при `--tray` спрятать окно.
        tray_hide_done: bool,
        /// Окно показали автоматически ради диалога подтверждения —
        /// после ответа его нужно вернуть в трей.
        shown_for_dialog: bool,
        /// Запущено ли с повышенными правами (иначе действует UIPI-ограничение).
        elevated: bool,
        /// Состояние службы «RatNext» (для окна настроек).
        #[cfg(feature = "ffmpeg")]
        svc_state: crate::service::State,
        /// Время последнего опроса состояния службы.
        #[cfg(feature = "ffmpeg")]
        svc_checked: Instant,
        /// Запрос «показать окно» от повторного запуска exe.
        show_request: Option<rat_session::single_instance::ShowListener>,
        /// Контекст egui для слушателя повторного запуска (будит перерисовку).
        repaint_slot: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>,
    }

    impl RatApp {
        fn new(
            args: Args,
            show_request: Option<rat_session::single_instance::ShowListener>,
            repaint_slot: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>,
        ) -> Self {
            let machines = Arc::new(Mutex::new(load()));
            let statuses: Arc<Mutex<HashMap<String, bool>>> = Arc::new(Mutex::new(HashMap::new()));
            {
                let machines = machines.clone();
                let statuses = statuses.clone();
                std::thread::spawn(move || {
                    loop {
                        let device = crate::store::load_device();
                        let list: Vec<String> = machines
                            .lock()
                            .unwrap()
                            .iter()
                            .map(|m| m.addr.clone())
                            .collect();
                        for addr in list {
                            let ok = match device.mode {
                                ConnectionMode::Lan => addr
                                    .to_socket_addrs()
                                    .ok()
                                    .and_then(|mut it| it.next())
                                    .map(|sa| {
                                        std::net::TcpStream::connect_timeout(
                                            &sa,
                                            Duration::from_millis(500),
                                        )
                                        .is_ok()
                                    })
                                    .unwrap_or(false),
                                ConnectionMode::Internet => {
                                    rat_session::link::relay_who(device.relay.as_str(), &addr)
                                        .unwrap_or(false)
                                }
                            };
                            statuses.lock().unwrap().insert(addr, ok);
                        }
                        std::thread::sleep(Duration::from_secs(4));
                    }
                });
            }
            let port = args.port;
            let (incoming_tx, incoming_rx) = channel();
            let mut server = ServerHandle::new(incoming_tx.clone());
            let mut toast = None;
            if let Err(e) = server.start(port) {
                crate::note(&format!("[ui] сервер не запустился: {e}"));
                toast = Some((format!("Сервер не запустился: {e}"), Instant::now()));
            } else {
                crate::note(&format!("[ui] сервер запущен, порт {port}"));
            }
            let monitors = rat_session::capture::list_outputs().unwrap_or_default();
            crate::note(&format!("[ui] мониторов найдено: {}", monitors.len()));
            let monitor_sel = args.monitor;
            server.monitor.store(monitor_sel, Ordering::Relaxed);
            let mut device = load_device();
            if let Some(id) = &args.device_id {
                device.id = id.clone();
            }
            if let Some(relay) = &args.relay {
                device.relay = normalize_relay(relay);
                device.mode = ConnectionMode::Internet;
            }
            device.relay = normalize_relay(&device.relay);
            if !is_valid_id(&device.id) {
                match gen_id() {
                    Ok(id) => device.id = id,
                    Err(error) => {
                        crate::note(&format!("[ui] не удалось создать ID: {error}"));
                        device.id.clear();
                        if toast.is_none() {
                            toast =
                                Some((format!("Не удалось создать ID: {error}"), Instant::now()));
                        }
                    }
                }
            }
            save_device(&device);
            let settings_mode = device.mode;
            let settings_relay_edit = device.relay.clone();
            let settings_auto_allow = device.auto_allow;
            let settings_tray = device.tray;
            crate::approval::set_auto_allow(device.auto_allow);
            // Служба («RatNext») — значок в трее создаём всегда: иначе к окну
            // скрытого агента не добраться. Настройка действует для обычного
            // запуска и автозапуска задачи, как раньше.
            let tray = if device.tray || args.service_helper {
                match crate::tray::Tray::new(!args.service_helper) {
                    Ok(tray) => Some(tray),
                    Err(e) => {
                        crate::note(&format!("[ui] значок в трее не создан: {e}"));
                        if toast.is_none() {
                            toast = Some((
                                format!("Не удалось создать значок в трее: {e}"),
                                Instant::now(),
                            ));
                        }
                        None
                    }
                }
            } else {
                None
            };
            let mut internet = InternetHandle::new(server.monitor.clone(), incoming_tx);
            if device.mode == ConnectionMode::Internet && is_valid_id(&device.id) {
                let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "rat-pc".into());
                internet.start(device.relay.clone(), device.id.clone(), name);
            }
            Self {
                machines,
                statuses,
                server,
                internet,
                device,
                settings_open: false,
                settings_mode,
                settings_relay_edit,
                settings_auto_allow,
                settings_tray,
                settings_autostart: false,
                settings_error: None,
                incoming_rx,
                incoming_pending: None,
                monitors,
                monitor_sel,
                port_edit: port.to_string(),
                my_ip: local_ip(),
                my_name: std::env::var("COMPUTERNAME").unwrap_or_default(),
                screen: Screen::Home,
                viewer: None,
                connect_rx: None,
                connecting_to: None,
                toast,
                add_open: false,
                add_name: String::new(),
                add_addr: String::new(),
                add_err: None,
                edit_open: None,
                edit_name: String::new(),
                autoconnect_done: false,
                started: Instant::now(),
                finished: false,
                tray,
                quitting: false,
                hidden: false,
                tray_hide_done: false,
                shown_for_dialog: false,
                elevated: rat_session::elevation::is_elevated(),
                #[cfg(feature = "ffmpeg")]
                svc_state: crate::service::State::NotInstalled,
                #[cfg(feature = "ffmpeg")]
                svc_checked: Instant::now(),
                show_request,
                repaint_slot,
                args,
            }
        }

        fn toast(&mut self, s: impl Into<String>) {
            self.toast = Some((s.into(), Instant::now()));
        }

        fn start_connect(&mut self, name: String, addr: String) {
            if self.viewer.is_some() || self.connect_rx.is_some() {
                return;
            }
            self.connecting_to = Some(addr.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            let addr_conn = addr.clone();
            let name_conn = name.clone();
            let switch_after = self.args.switch_after;
            let relay_conn = self.device.relay.clone();
            let internet = self.device.mode == ConnectionMode::Internet;
            std::thread::spawn(move || {
                let mut connected = None;
                let mut last_err = String::new();
                for attempt in 0..10 {
                    let r = if internet {
                        if relay_conn.trim().is_empty() {
                            Err(std::io::Error::other("не задан сервер (Интернет-режим)"))
                        } else {
                            rat_session::link::relay_connect(relay_conn.as_str(), &addr_conn)
                        }
                    } else {
                        tcp_connect(addr_conn.as_str())
                    };
                    match r {
                        Ok(v) => {
                            connected = Some(v);
                            break;
                        }
                        Err(e) => {
                            last_err = e.to_string();
                            if attempt < 9 {
                                std::thread::sleep(Duration::from_millis(300));
                            }
                        }
                    }
                }
                let res = match connected {
                    Some((link, closer)) => {
                        let stats = Arc::new(Mutex::new(SessionStats::default()));
                        let stop = Arc::new(AtomicBool::new(false));
                        let base = TimeBase(Instant::now());
                        let (ui_tx, ui_rx) = sync_channel::<FrameOut>(3);
                        let input_q: Arc<Mutex<Vec<InputEvent>>> = Arc::new(Mutex::new(Vec::new()));
                        let cursor = Arc::new(Mutex::new(CursorState::default()));
                        let info: Arc<Mutex<Option<SessionInfo>>> = Arc::new(Mutex::new(None));
                        let monitors: Arc<Mutex<Vec<(u16, u16, u16)>>> =
                            Arc::new(Mutex::new(Vec::new()));
                        let pending_switch: Arc<Mutex<Option<u16>>> = Arc::new(Mutex::new(None));
                        let clip_in: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
                        let pending_paste: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
                        let handle = spawn_viewer(
                            base,
                            stop.clone(),
                            stats.clone(),
                            link,
                            ui_tx,
                            ViewerShared {
                                input_q: input_q.clone(),
                                cursor: cursor.clone(),
                                info: info.clone(),
                                monitors: monitors.clone(),
                                pending_switch: pending_switch.clone(),
                                clip_in: clip_in.clone(),
                                pending_paste: pending_paste.clone(),
                            },
                        );
                        Ok(ViewerSession {
                            name: name_conn,
                            addr: addr_conn,
                            rx: ui_rx,
                            stats,
                            stop,
                            handle: Some(handle),
                            closer,
                            base,
                            texture: None,
                            last: None,
                            input_q,
                            cursor,
                            info,
                            monitors,
                            pending_switch,
                            clip_in,
                            pending_paste,
                            last_clip_applied: None,
                            top_open: false,
                            bottom_open: false,
                            top_t: 0.0,
                            bottom_t: 0.0,
                            pinned: false,
                            switch_after,
                            switch_done: false,
                            connected_at: Instant::now(),
                            last_pointer: None,
                            prev_mods: egui::Modifiers::default(),
                            buttons_down: [false; 5],
                            ui_e2e_ms: Vec::new(),
                        })
                    }
                    None => Err(format!("не удалось подключиться к {addr_conn}: {last_err}")),
                };
                let _ = tx.send(res);
            });
            self.connect_rx = Some(rx);
        }

        fn poll_connect(&mut self) {
            let Some(rx) = self.connect_rx.as_ref() else {
                return;
            };
            let res = rx.try_recv();
            match res {
                Ok(Ok(session)) => {
                    self.connecting_to = None;
                    self.connect_rx = None;
                    crate::note(&format!("[ui] подключено: {}", session.addr));
                    {
                        let mut list = self.machines.lock().unwrap();
                        if let Some(m) = list.iter_mut().find(|m| m.addr == session.addr) {
                            m.last_connected = Some(now_unix());
                        }
                        save(&list);
                    }
                    self.viewer = Some(session);
                    self.screen = Screen::Viewer;
                }
                Ok(Err(e)) => {
                    self.connecting_to = None;
                    self.connect_rx = None;
                    crate::note(&format!("[ui] ошибка подключения: {e}"));
                    self.toast(e);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.connecting_to = None;
                    self.connect_rx = None;
                }
            }
        }

        fn home_ui(&mut self, ui: &mut egui::Ui) {
            egui::Panel::top("top").show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.heading("Rat Next");
                    ui.separator();
                    let running = self.server.is_running();
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(rect.center(), 5.0, if running { ON } else { OFF });
                    // Одна кнопка на оба состояния: надпись меняется по факту.
                    if running {
                        if ui.button("Остановить сервер").clicked() {
                            self.server.stop();
                        }
                    } else {
                        ui.label("Порт:");
                        ui.add(egui::TextEdit::singleline(&mut self.port_edit).desired_width(56.0));
                        if ui.button("Запустить сервер").clicked() {
                            match self.port_edit.trim().parse::<u16>() {
                                Ok(p) => {
                                    if let Err(e) = self.server.start(p) {
                                        self.toast(format!("Сервер не запустился: {e}"));
                                    }
                                }
                                Err(_) => self.toast("Некорректный порт"),
                            }
                        }
                    }
                    ui.separator();
                    if self.my_name.is_empty() {
                        ui.label(format!("{}:{}", self.my_ip, self.server.port));
                    } else {
                        ui.label(format!(
                            "{} ({}:{})",
                            self.my_name, self.my_ip, self.server.port
                        ));
                    }
                    ui.separator();
                    if !self.monitors.is_empty() {
                        ui.label("Монитор:");
                        let mut pick: Option<u32> = None;
                        for m in &self.monitors {
                            let selected = self.monitor_sel == m.index;
                            let txt = format!("{} ({}×{})", m.index + 1, m.width, m.height);
                            if ui.selectable_label(selected, txt).clicked() {
                                pick = Some(m.index);
                            }
                        }
                        if let Some(idx) = pick
                            && idx != self.monitor_sel
                        {
                            self.monitor_sel = idx;
                            self.server.monitor.store(idx, Ordering::Relaxed);
                            self.toast("Сменил монитор — применится к следующему подключению");
                        }
                    }
                    ui.label(format!(
                        "сессий: {}",
                        self.server.sessions.load(Ordering::Relaxed)
                    ));
                    if let Some(e) = self.server.last_err.lock().unwrap().clone() {
                        ui.separator();
                        ui.colored_label(OFF, format!("ошибка: {e}"));
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    // «Мой ID» слева: подпись — по высоте номера, номер — жирный.
                    let id_font = egui::FontId::new(34.0, egui::FontFamily::Name(ID_BOLD.into()));
                    // Резервируем высоту строки заранее — иначе подпись
                    // центрируется по ещё «короткой» строке и уезжает вверх.
                    let line_h = ui.ctx().fonts_mut(|f| f.row_height(&id_font));
                    ui.set_min_height(line_h);
                    ui.vertical(|ui| {
                        // Подпись — по оптическому центру цифр номера
                        // (18px выверены по пикселям; при смене кегля подстроить).
                        ui.add_space(18.0);
                        ui.label(egui::RichText::new("Мой ID").color(ID_COLOR));
                    });
                    ui.label(
                        egui::RichText::new(self.device.id.clone())
                            .font(id_font)
                            .color(ID_BRIGHT),
                    );
                    if ui.button("копировать").clicked() {
                        ui.ctx().copy_text(self.device.id.clone());
                        self.toast("ID скопирован в буфер обмена");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Настройки").clicked() {
                            self.settings_mode = self.device.mode;
                            self.settings_relay_edit = self.device.relay.clone();
                            self.settings_auto_allow = self.device.auto_allow;
                            self.settings_tray = self.device.tray;
                            self.settings_autostart = crate::autostart::is_installed();
                            self.settings_error = None;
                            self.settings_open = true;
                        }
                    });
                });
                if !self.elevated && !self.args.service_helper {
                    ui.horizontal(|ui| {
                        ui.colored_label(OFF, "⚠");
                        ui.label(
                            egui::RichText::new(
                                "нет прав администратора — управление окнами программ, \
                                 запущенных от администратора, недоступно",
                            )
                            .size(12.0)
                            .color(egui::Color32::from_rgb(180, 83, 9)),
                        )
                        .on_hover_text(
                            "Перезапустите от имени администратора, включите автозапуск \
                             (задача с повышенными правами) или установите службу — \
                             тогда мышь и клавиатура работают в любых окнах.",
                        );
                    });
                }
                ui.add_space(6.0);
            });
            egui::Panel::bottom("bottom").show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("+ Добавить машину").clicked() {
                        self.add_open = true;
                        self.add_err = None;
                    }
                    if let Some(a) = &self.connecting_to {
                        ui.separator();
                        ui.spinner();
                        ui.label(format!("Подключение к {a}…"));
                    }
                    if let Some((msg, at)) = &self.toast
                        && at.elapsed().as_secs_f32() < 6.0
                    {
                        ui.separator();
                        ui.colored_label(egui::Color32::from_rgb(255, 200, 80), msg);
                    }
                });
                ui.horizontal(|ui| {
                    let mode_label = match self.device.mode {
                        ConnectionMode::Internet => "Интернет",
                        ConnectionMode::Lan => "локальная сеть",
                    };
                    ui.label(format!("Режим: {mode_label}"));
                    if self.device.mode == ConnectionMode::Internet {
                        let running = self.internet.is_running();
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(
                            rect.center(),
                            5.0,
                            if running { ON } else { OFF },
                        );
                        ui.label(self.internet.status.lock().unwrap().clone());
                    }
                });
                ui.add_space(6.0);
            });

            let snapshot: Vec<(usize, Machine, bool)> = {
                let list = self.machines.lock().unwrap();
                let st = self.statuses.lock().unwrap();
                list.iter()
                    .enumerate()
                    .map(|(i, m)| (i, m.clone(), st.get(&m.addr).copied().unwrap_or(false)))
                    .collect()
            };
            let mut actions: Vec<TileAction> = Vec::new();
            egui::CentralPanel::default().show(ui, |ui| {
                if snapshot.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label("Пока нет машин.");
                        ui.label("Нажмите «+ Добавить машину» внизу, чтобы добавить первую.");
                        ui.label("Подключение — по адресу IP:порт (например 192.168.1.50:55140).");
                    });
                    return;
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (i, m, online) in &snapshot {
                            let addr = m.addr.clone();
                            let resp = tile(ui, m, *online);
                            if resp.clicked() {
                                actions.push(TileAction::Connect(*i));
                            }
                            resp.context_menu(|ui| {
                                if ui.button("Подключиться").clicked() {
                                    actions.push(TileAction::Connect(*i));
                                    ui.close();
                                }
                                if ui.button("Переименовать").clicked() {
                                    actions.push(TileAction::Rename(*i));
                                    ui.close();
                                }
                                if ui.button("Скопировать адрес").clicked() {
                                    ui.ctx().copy_text(addr.clone());
                                    ui.close();
                                }
                                if ui.button("Удалить").clicked() {
                                    actions.push(TileAction::Delete(*i));
                                    ui.close();
                                }
                            });
                        }
                    });
                });
            });
            for a in actions {
                match a {
                    TileAction::Connect(i) => {
                        let pair = {
                            let list = self.machines.lock().unwrap();
                            list.get(i).map(|m| (m.name.clone(), m.addr.clone()))
                        };
                        if let Some((name, addr)) = pair {
                            self.start_connect(name, addr);
                        }
                    }
                    TileAction::Rename(i) => {
                        let name = {
                            let list = self.machines.lock().unwrap();
                            list.get(i).map(|m| m.name.clone())
                        };
                        if let Some(n) = name {
                            self.edit_name = n;
                            self.edit_open = Some(i);
                        }
                    }
                    TileAction::Delete(i) => {
                        let mut list = self.machines.lock().unwrap();
                        if i < list.len() {
                            list.remove(i);
                        }
                        save(&list);
                    }
                }
            }
        }

        fn dialogs(&mut self, ctx: &egui::Context) {
            if self.incoming_pending.is_none()
                && let Ok(request) = self.incoming_rx.try_recv()
            {
                self.incoming_pending = Some(request);
                crate::notify::beep();
                // Если окно скрыто в трее — показать его, иначе запрос не увидеть.
                if self.hidden {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.hidden = false;
                    self.shown_for_dialog = true;
                }
            }
            let request_expired = self
                .incoming_pending
                .as_ref()
                .is_some_and(|request| request.expires_at <= Instant::now());
            if request_expired && let Some(request) = self.incoming_pending.take() {
                let _ = request.reply.send(false);
            }

            let mut approval_decision = None;
            if let Some(request) = self.incoming_pending.as_ref() {
                let mut open = true;
                egui::Window::new("Подтверждение подключения")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .open(&mut open)
                    .show(ctx, |ui| {
                        let source = match request.source {
                            Source::Lan => "локальной сети",
                            Source::Internet => "Интернета через relay",
                        };
                        ui.label(format!("Запрос на подключение из {source}."));
                        ui.label("Будет открыт удалённый просмотр экрана и управление.");
                        ui.label("Разрешить эту сессию?");
                        ui.horizontal(|ui| {
                            if ui.button("Разрешить").clicked() {
                                approval_decision = Some(true);
                            }
                            if ui.button("Отклонить").clicked() {
                                approval_decision = Some(false);
                            }
                        });
                    });
                if !open && approval_decision.is_none() {
                    approval_decision = Some(false);
                }
            }
            if let Some(allowed) = approval_decision
                && let Some(request) = self.incoming_pending.take()
            {
                let _ = request.reply.send(allowed);
            }
            // Если окно разбудили только ради подтверждения — вернуть его в трей.
            if self.incoming_pending.is_none() && self.shown_for_dialog {
                self.shown_for_dialog = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.hidden = true;
            }

            if self.settings_open {
                // Служба: состояние опрашиваем не чаще раза в секунду.
                #[cfg(feature = "ffmpeg")]
                if self.svc_checked.elapsed() > std::time::Duration::from_secs(1) {
                    self.svc_checked = Instant::now();
                    self.svc_state = crate::service::status();
                }
                let mut open = true;
                let mut apply = false;
                let mut cancel = false;
                egui::Window::new("Настройки подключения")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label("Режим подключения:");
                        ui.radio_value(
                            &mut self.settings_mode,
                            ConnectionMode::Internet,
                            "Интернет через relay",
                        );
                        ui.radio_value(
                            &mut self.settings_mode,
                            ConnectionMode::Lan,
                            "Локальная сеть (IP:порт)",
                        );
                        if self.settings_mode == ConnectionMode::Internet {
                            ui.separator();
                            ui.label("Адрес relay (host:порт):");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings_relay_edit)
                                    .desired_width(280.0),
                            );
                        }
                        ui.separator();
                        ui.checkbox(
                            &mut self.settings_auto_allow,
                            "Подключать без подтверждения",
                        );
                        ui.label(
                            egui::RichText::new(
                                "Подключения принимаются сразу — и в локальной сети, и через relay",
                            )
                            .small()
                            .color(egui::Color32::from_gray(150)),
                        );
                        ui.checkbox(&mut self.settings_tray, "Сворачивать в трей");
                        ui.checkbox(
                            &mut self.settings_autostart,
                            "Запускать при входе в Windows (сразу в трее)",
                        );
                        #[cfg(feature = "ffmpeg")]
                        {
                            ui.separator();
                            ui.label("Служба Windows — агент доступен с загрузки системы:");
                            let (state_txt, state_col) = match self.svc_state {
                                crate::service::State::NotInstalled => {
                                    ("не установлена", egui::Color32::from_gray(130))
                                }
                                crate::service::State::Stopped => {
                                    ("остановлена", egui::Color32::from_rgb(230, 120, 30))
                                }
                                crate::service::State::Running => ("работает", ON),
                                crate::service::State::Pending => {
                                    ("меняется…", egui::Color32::from_rgb(150, 150, 60))
                                }
                            };
                            ui.horizontal(|ui| {
                                ui.label("Состояние:");
                                ui.colored_label(state_col, state_txt);
                            });
                            let not_installed =
                                matches!(self.svc_state, crate::service::State::NotInstalled);
                            let running = matches!(self.svc_state, crate::service::State::Running);
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(not_installed, egui::Button::new("Установить"))
                                    .clicked()
                                {
                                    self.svc_action(crate::service::Op::Install);
                                }
                                if ui
                                    .add_enabled(
                                        !not_installed && !running,
                                        egui::Button::new("Запустить"),
                                    )
                                    .clicked()
                                {
                                    self.svc_action(crate::service::Op::Start);
                                }
                                if ui
                                    .add_enabled(
                                        !not_installed && running,
                                        egui::Button::new("Остановить"),
                                    )
                                    .clicked()
                                {
                                    self.svc_action(crate::service::Op::Stop);
                                }
                                if ui
                                    .add_enabled(!not_installed, egui::Button::new("Удалить"))
                                    .clicked()
                                {
                                    self.svc_action(crate::service::Op::Uninstall);
                                }
                            });
                            ui.label(
                                egui::RichText::new(
                                    "Для установки, удаления и остановки Windows \
                                     запросит права администратора.",
                                )
                                .small()
                                .color(egui::Color32::from_gray(150)),
                            );
                        }
                        if let Some(error) = &self.settings_error {
                            ui.colored_label(OFF, error);
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Сохранить").clicked() {
                                apply = true;
                            }
                            if ui.button("Отмена").clicked() {
                                cancel = true;
                            }
                        });
                    });
                if apply {
                    let relay = normalize_relay(&self.settings_relay_edit);
                    let valid_relay = relay.rsplit_once(':').is_some_and(|(host, port)| {
                        !host.trim().is_empty()
                            && port.parse::<u16>().is_ok_and(|parsed| parsed > 0)
                    });
                    if self.settings_mode == ConnectionMode::Internet && !valid_relay {
                        self.settings_error = Some(
                            "Укажите адрес relay в формате host:порт, например 194.37.254.233:8889"
                                .to_string(),
                        );
                    } else {
                        let old_mode = self.device.mode;
                        let old_relay = self.device.relay.clone();
                        let restart = old_mode != self.settings_mode
                            || old_relay != relay
                            || !self.internet.is_running();
                        self.device.mode = self.settings_mode;
                        self.device.relay = relay;
                        self.device.auto_allow = self.settings_auto_allow;
                        crate::approval::set_auto_allow(self.settings_auto_allow);
                        if self.settings_tray != self.device.tray {
                            if self.settings_tray {
                                match crate::tray::Tray::new(true) {
                                    Ok(tray) => {
                                        self.tray = Some(tray);
                                        self.device.tray = true;
                                    }
                                    Err(e) => {
                                        self.settings_tray = false;
                                        self.device.tray = false;
                                        self.toast(format!("Значок в трее недоступен: {e}"));
                                    }
                                }
                            } else {
                                self.tray = None;
                                self.device.tray = false;
                            }
                        }
                        let autostart_now = crate::autostart::is_installed();
                        if self.settings_autostart != autostart_now {
                            let result = if self.settings_autostart {
                                crate::autostart::install()
                            } else {
                                crate::autostart::remove()
                            };
                            match result {
                                Ok(()) => self.toast(if self.settings_autostart {
                                    "Автозапуск включён"
                                } else {
                                    "Автозапуск выключен"
                                }),
                                Err(e) => {
                                    self.settings_autostart = autostart_now;
                                    self.toast(format!("Автозапуск: {e}"));
                                }
                            }
                        }
                        save_device(&self.device);
                        self.settings_open = false;
                        self.settings_error = None;
                        if restart {
                            self.internet.stop();
                        }
                        if self.device.mode == ConnectionMode::Internet && restart {
                            if is_valid_id(&self.device.id) {
                                let name = if self.my_name.is_empty() {
                                    "rat-pc".to_string()
                                } else {
                                    self.my_name.clone()
                                };
                                self.internet.start(
                                    self.device.relay.clone(),
                                    self.device.id.clone(),
                                    name,
                                );
                            } else {
                                self.toast("Не удалось создать шестизначный ID");
                            }
                        }
                    }
                } else if cancel || !open {
                    self.settings_open = false;
                    self.settings_error = None;
                }
            }

            if self.add_open {
                let mut open = self.add_open;
                egui::Window::new("Добавить машину")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label("Имя машины:");
                        ui.add(egui::TextEdit::singleline(&mut self.add_name).desired_width(280.0));
                        let address_hint = match self.device.mode {
                            ConnectionMode::Internet => "ID устройства (6 цифр):",
                            ConnectionMode::Lan => "Адрес машины (IP:порт):",
                        };
                        ui.label(address_hint);
                        ui.add(egui::TextEdit::singleline(&mut self.add_addr).desired_width(280.0));
                        if let Some(e) = &self.add_err {
                            ui.colored_label(OFF, e);
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Добавить").clicked() {
                                let addr = match self.device.mode {
                                    ConnectionMode::Internet => self.add_addr.trim().to_string(),
                                    ConnectionMode::Lan => normalize_addr(&self.add_addr),
                                };
                                let ok = match self.device.mode {
                                    ConnectionMode::Internet => is_valid_id(&addr),
                                    ConnectionMode::Lan => {
                                        let (host, port) = split_addr(&addr);
                                        !host.is_empty() && port.is_some_and(|value| value > 0)
                                    }
                                };
                                if !ok {
                                    self.add_err = Some(match self.device.mode {
                                        ConnectionMode::Internet => {
                                            "Введите ID ровно из 6 цифр".into()
                                        }
                                        ConnectionMode::Lan => {
                                            "Введите адрес машины в формате IP:порт".into()
                                        }
                                    });
                                } else {
                                    let mut list = self.machines.lock().unwrap();
                                    if list.iter().any(|m| m.addr == addr) {
                                        self.add_err =
                                            Some("Такая машина уже есть в списке".into());
                                    } else {
                                        let name = if self.add_name.trim().is_empty() {
                                            addr.clone()
                                        } else {
                                            self.add_name.trim().to_string()
                                        };
                                        list.push(Machine {
                                            name,
                                            addr,
                                            last_connected: None,
                                        });
                                        save(&list);
                                        drop(list);
                                        self.add_err = None;
                                        self.add_open = false;
                                        self.add_name.clear();
                                        self.add_addr.clear();
                                    }
                                }
                            }
                            if ui.button("Отмена").clicked() {
                                self.add_open = false;
                            }
                        });
                    });
                self.add_open = open && self.add_open;
            }
            if let Some(i) = self.edit_open {
                let mut open = true;
                egui::Window::new("Переименовать машину")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_name).desired_width(280.0),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Сохранить").clicked() {
                                {
                                    let mut list = self.machines.lock().unwrap();
                                    if let Some(m) = list.get_mut(i) {
                                        let nm = self.edit_name.trim();
                                        if !nm.is_empty() {
                                            m.name = nm.to_string();
                                        }
                                    }
                                    save(&list);
                                }
                                self.edit_open = None;
                            }
                            if ui.button("Отмена").clicked() {
                                self.edit_open = None;
                            }
                        });
                    });
                if !open {
                    self.edit_open = None;
                }
            }
        }
    }

    fn tile(ui: &mut egui::Ui, m: &Machine, online: bool) -> egui::Response {
        let desired = egui::vec2(225.0, 102.0);
        let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
        let border = if resp.hovered() {
            CARD_BORDER_HOVER
        } else {
            CARD_BORDER
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 8.0, border);
        painter.rect_filled(rect.shrink(1.5), 7.0, CARD_BG);
        painter.circle_filled(
            egui::pos2(rect.left() + 18.0, rect.top() + 20.0),
            5.0,
            if online { ON } else { OFF },
        );
        painter.text(
            egui::pos2(rect.left() + 32.0, rect.top() + 12.0),
            egui::Align2::LEFT_TOP,
            &m.name,
            egui::FontId::proportional(16.0),
            egui::Color32::from_rgb(238, 240, 244),
        );
        painter.text(
            egui::pos2(rect.left() + 18.0, rect.top() + 46.0),
            egui::Align2::LEFT_TOP,
            &m.addr,
            egui::FontId::monospace(12.0),
            DIM,
        );
        let last = m
            .last_connected
            .map(ago_text)
            .unwrap_or_else(|| "ещё не подключались".into());
        painter.text(
            egui::pos2(rect.left() + 18.0, rect.top() + 72.0),
            egui::Align2::LEFT_TOP,
            &last,
            egui::FontId::proportional(11.0),
            DIM2,
        );
        resp
    }

    fn local_ip() -> String {
        std::net::UdpSocket::bind("0.0.0.0:0")
            .and_then(|s| {
                s.connect("8.8.8.8:80")?;
                s.local_addr().map(|a| a.ip().to_string())
            })
            .unwrap_or_else(|_| "?".into())
    }

    fn split_addr(s: &str) -> (String, Option<u16>) {
        match s.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().ok()),
            None => (s.to_string(), None),
        }
    }
    impl RatApp {
        fn viewer_ui(&mut self, ui: &mut egui::Ui) {
            let mut back = false;
            let Some(v) = self.viewer.as_mut() else {
                self.screen = Screen::Home;
                return;
            };
            loop {
                match v.rx.try_recv() {
                    Ok(f) => {
                        let now = rat_session::stats::now_us(&v.base);
                        let e = ((now as i64 - f.t_capture_us as i64).max(0)) as f32 / 1000.0;
                        SessionStats::push_sample(&mut v.ui_e2e_ms, e);
                        v.last = Some(f);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break,
                }
            }
            if let Some(f) = &v.last {
                let img = egui::ColorImage::from_rgba_unmultiplied([f.w, f.h], &f.rgba);
                let data = egui::ImageData::Color(std::sync::Arc::new(img));
                match &mut v.texture {
                    Some(t) => t.set(data, egui::TextureOptions::LINEAR),
                    None => {
                        v.texture = Some(ui.ctx().load_texture(
                            "video",
                            data,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
            }
            // Тестовый авто-переключатель монитора (--switch-after=СЕК).
            if let Some(sec) = v.switch_after
                && !v.switch_done
                && v.connected_at.elapsed().as_secs_f32() >= sec
            {
                v.switch_done = true;
                let ids: Vec<u16> = v.monitors.lock().unwrap().iter().map(|m| m.0).collect();
                if !ids.is_empty() {
                    let cur = v
                        .info
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|s| s.active)
                        .unwrap_or(ids[0]);
                    let next = ids.iter().copied().find(|&i| i != cur).unwrap_or(cur);
                    *v.pending_switch.lock().unwrap() = Some(next);
                    crate::note(&format!("[ui] авто-переключение монитора -> {next}"));
                }
            }
            // Локальный буфер: текст, скопированный на удалённой машине.
            let clip_now = v.clip_in.lock().unwrap().clone();
            if clip_now != v.last_clip_applied {
                if let Some(text) = &clip_now {
                    ui.ctx().copy_text(text.clone());
                }
                v.last_clip_applied = clip_now;
            }
            // ---- панели-оверлеи: прячутся, выезжают при наведении к краю ----
            const TOP_H: f32 = 42.0;
            const BOT_H: f32 = 30.0;
            const EDGE: f32 = 6.0;
            let screen = ui.ctx().content_rect();
            let (ptr0, any_down, dt) = ui.ctx().input(|i| {
                (
                    i.pointer.hover_pos(),
                    i.pointer.any_down(),
                    i.stable_dt.clamp(0.001, 0.05),
                )
            });
            let in_win = |p: egui::Pos2| screen.expand(4.0).contains(p);
            let want_top = v.pinned
                || (!any_down && ptr0.is_some_and(|p| in_win(p) && p.y <= screen.top() + EDGE))
                || (v.top_open
                    && ptr0.is_some_and(|p| in_win(p) && p.y <= screen.top() + TOP_H + 4.0));
            let want_bottom = v.pinned
                || (!any_down && ptr0.is_some_and(|p| in_win(p) && p.y >= screen.bottom() - EDGE))
                || (v.bottom_open
                    && ptr0.is_some_and(|p| in_win(p) && p.y >= screen.bottom() - BOT_H - 4.0));
            v.top_open = want_top;
            v.bottom_open = want_bottom;
            for (t, open, dur_in, dur_out) in [
                (&mut v.top_t, v.top_open, 0.35f32, 0.20f32),
                (&mut v.bottom_t, v.bottom_open, 0.30f32, 0.18f32),
            ] {
                let target = if open { 1.0 } else { 0.0 };
                let dur = if target > *t { dur_in } else { dur_out };
                let step = dt / dur;
                *t = if (target - *t).abs() <= step {
                    target
                } else {
                    *t + (target - *t).signum() * step
                };
            }
            // Светлая палитра — как у остального интерфейса: тёмный текст на
            // светлом фоне читается и поверх чёрных полей, и поверх видео.
            let bar_fill = egui::Color32::from_rgba_unmultiplied(243, 245, 248, 240);
            let bar_border = egui::Color32::from_rgb(196, 203, 213);
            if v.top_t > 0.01 {
                let y = screen.top() - TOP_H * (1.0 - v.top_t);
                egui::Area::new(egui::Id::new("vt_top"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(egui::pos2(screen.left(), y))
                    .interactable(true)
                    .show(ui.ctx(), |ui| {
                        let bg = egui::Rect::from_min_size(
                            egui::pos2(screen.left(), y),
                            egui::vec2(screen.width(), TOP_H),
                        );
                        ui.painter().rect_filled(bg, 0.0, bar_fill);
                        ui.painter().rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(screen.left(), y + TOP_H - 1.0),
                                egui::pos2(screen.right(), y + TOP_H),
                            ),
                            0.0,
                            bar_border,
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(screen.width(), TOP_H),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    let disconnect = egui::Button::new(
                                        egui::RichText::new("⬅")
                                            .size(24.0)
                                            .color(egui::Color32::from_rgb(219, 56, 52)),
                                    )
                                    .fill(egui::Color32::from_rgb(255, 236, 234));
                                    if ui
                                        .add(disconnect)
                                        .on_hover_text("Отключиться от удалённого ПК")
                                        .clicked()
                                    {
                                        back = true;
                                    }
                                    ui.separator();
                                    ui.label(format!("{} ({})", v.name, v.addr));
                                    let st = v.stats.lock().unwrap();
                                    ui.separator();
                                    ui.label(format!("fps: {:.1}", st.fps));
                                    ui.separator();
                                    ui.label(format!("e2e {:.0}ms", SessionStats::avg(&st.e2e_ms)));
                                    ui.separator();
                                    ui.label(format!("sent {} | shown {}", st.sent_pkts, st.shown));
                                    if !st.last_error.is_empty() {
                                        ui.separator();
                                        ui.colored_label(OFF, format!("ошибка: {}", st.last_error));
                                    }
                                    ui.separator();
                                    let mons = v.monitors.lock().unwrap().clone();
                                    let cur = v
                                        .info
                                        .lock()
                                        .unwrap()
                                        .as_ref()
                                        .map(|s| s.active)
                                        .unwrap_or(0);
                                    if !mons.is_empty() {
                                        ui.label("🖥").on_hover_text("Монитор — переключение");
                                        for (idx, mw, mh) in &mons {
                                            let sel = cur == *idx;
                                            if ui
                                                .selectable_label(
                                                    sel,
                                                    format!("{} ({}×{})", idx + 1, mw, mh),
                                                )
                                                .clicked()
                                                && !sel
                                            {
                                                *v.pending_switch.lock().unwrap() = Some(*idx);
                                            }
                                        }
                                    } else if let Some(si) = v.info.lock().unwrap().as_ref() {
                                        ui.label(format!(
                                            "монитор {}×{}",
                                            si.out.width, si.out.height
                                        ));
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.add_space(10.0);
                                            let is_fs = ui
                                                .ctx()
                                                .input(|i| i.viewport().fullscreen)
                                                .unwrap_or(false);
                                            let (label, hint) = if is_fs {
                                                ("🗗", "Вернуть в окно")
                                            } else {
                                                ("⛶", "Во весь экран")
                                            };
                                            if ui.button(label).on_hover_text(hint).clicked() {
                                                ui.ctx().send_viewport_cmd(
                                                    egui::ViewportCommand::Fullscreen(!is_fs),
                                                );
                                            }
                                            if ui
                                                .selectable_label(v.pinned, "📌")
                                                .on_hover_text(
                                                    "Закрепить панели — не скрывать автоматически",
                                                )
                                                .clicked()
                                            {
                                                v.pinned = !v.pinned;
                                            }
                                        },
                                    );
                                });
                            },
                        );
                    });
            }
            if v.bottom_t > 0.01 {
                let y = screen.bottom() - BOT_H * v.bottom_t;
                egui::Area::new(egui::Id::new("vt_bottom"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(egui::pos2(screen.left(), y))
                    .interactable(true)
                    .show(ui.ctx(), |ui| {
                        let bg = egui::Rect::from_min_size(
                            egui::pos2(screen.left(), y),
                            egui::vec2(screen.width(), BOT_H),
                        );
                        ui.painter().rect_filled(bg, 0.0, bar_fill);
                        ui.painter().rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(screen.left(), y),
                                egui::pos2(screen.right(), y + 1.0),
                            ),
                            0.0,
                            bar_border,
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(screen.width(), BOT_H),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add_space(10.0);
                                ui.label(
                                    "Мышь — над видео; клавиатура — когда окно в фокусе. Панели выезжают при наведении к верхнему/нижнему краю. Кнопка со стрелкой ⬅ или крестик окна — отключиться.",
                                );
                            },
                        );
                    });
            }

            let mut video_rect: Option<egui::Rect> = None;
            egui::CentralPanel::default().show(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                ui.painter()
                    .rect_filled(rect, 2.0, egui::Color32::from_rgb(12, 14, 18));
                if let (Some(tex), Some(f)) = (&v.texture, &v.last) {
                    let img_ar = f.w as f32 / f.h as f32;
                    let rect_ar = rect.width() / rect.height().max(1.0);
                    let draw = if img_ar > rect_ar {
                        let h = rect.width() / img_ar;
                        egui::Rect::from_min_size(
                            egui::pos2(rect.min.x, rect.min.y + (rect.height() - h) / 2.0),
                            egui::vec2(rect.width(), h),
                        )
                    } else {
                        let w = rect.height() * img_ar;
                        egui::Rect::from_min_size(
                            egui::pos2(rect.min.x + (rect.width() - w) / 2.0, rect.min.y),
                            egui::vec2(w, rect.height()),
                        )
                    };
                    ui.painter().image(
                        tex.id(),
                        draw,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    video_rect = Some(draw);
                } else {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "ожидание кадров…",
                        egui::FontId::proportional(22.0),
                        egui::Color32::from_gray(150),
                    );
                }
            });

            // ---- ввод и удалённый курсор ----
            let input_q = v.input_q.clone();
            let cursor_arc = v.cursor.clone();
            if let Some(draw) = video_rect {
                let resp = ui.allocate_rect(draw, egui::Sense::click_and_drag());
                let mut locked = input_q.lock().unwrap();
                let focused = ui.ctx().input(|i| i.focused);

                // Панель поверх видео: пока указатель над ней, ввод в сессию не шлём.
                let over_bars = ptr0.is_some_and(|p| {
                    p.y <= screen.top() + TOP_H * v.top_t + 1.0
                        || p.y >= screen.bottom() - BOT_H * v.bottom_t - 1.0
                });

                // Указатель: движение, кнопки, колесо.
                let engaged = ((resp.hovered() || resp.dragged()) && !over_bars)
                    || v.buttons_down.iter().any(|d| *d);
                if engaged {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                    if !over_bars && let Some(p) = ui.ctx().input(|i| i.pointer.latest_pos()) {
                        let rel = p - draw.min;
                        let (nx, ny) =
                            input::ui_px_to_norm(rel.x, rel.y, draw.width(), draw.height());
                        if v.last_pointer != Some((nx, ny)) {
                            v.last_pointer = Some((nx, ny));
                            locked.push(InputEvent::Move { x: nx, y: ny });
                        }
                    }
                    let (mx, my) = v.last_pointer.unwrap_or((32768, 32768));
                    let mut pb: Vec<(u8, bool, f32, f32)> = Vec::new();
                    let mut wheel = (0f32, 0f32);
                    ui.ctx().input(|inp| {
                        wheel = (inp.smooth_scroll_delta.x, inp.smooth_scroll_delta.y);
                        for ev in &inp.events {
                            if let egui::Event::PointerButton {
                                pos,
                                button,
                                pressed,
                                ..
                            } = ev
                            {
                                let idx = match button {
                                    egui::PointerButton::Primary => 0u8,
                                    egui::PointerButton::Secondary => 1,
                                    egui::PointerButton::Middle => 2,
                                    egui::PointerButton::Extra1 => 3,
                                    egui::PointerButton::Extra2 => 4,
                                };
                                let rel = *pos - draw.min;
                                pb.push((idx, *pressed, rel.x, rel.y));
                            }
                        }
                    });
                    for (idx, pressed, rx, ry) in pb {
                        let i = idx as usize;
                        if (resp.hovered() && !over_bars) || v.buttons_down[i] {
                            let (nx, ny) =
                                input::ui_px_to_norm(rx, ry, draw.width(), draw.height());
                            v.last_pointer = Some((nx, ny));
                            v.buttons_down[i] = pressed;
                            locked.push(InputEvent::Button {
                                btn: idx,
                                down: pressed,
                                x: nx,
                                y: ny,
                            });
                        }
                    }
                    if resp.hovered() && !over_bars && (wheel.0.abs() > 0.5 || wheel.1.abs() > 0.5)
                    {
                        locked.push(InputEvent::Wheel {
                            dx: wheel.0.clamp(-32768.0, 32767.0) as i16,
                            dy: wheel.1.clamp(-32768.0, 32767.0) as i16,
                            x: mx,
                            y: my,
                        });
                    }
                }

                // Клавиатура: пока окно в фокусе.
                if focused {
                    let mut keys: Vec<(u16, bool)> = Vec::new();
                    let mut texts: Vec<String> = Vec::new();
                    let mut specials: Vec<u8> = Vec::new();
                    let mut paste_text: Option<String> = None;
                    ui.ctx().input(|inp| {
                        for ev in &inp.events {
                            match ev {
                                egui::Event::Key {
                                    key,
                                    pressed,
                                    modifiers,
                                    ..
                                } => {
                                    if let Some(vk) = vk_of_key(*key) {
                                        let shortcut =
                                            modifiers.ctrl || modifiers.alt || modifiers.mac_cmd;
                                        if shortcut || !is_char_vk(vk) {
                                            keys.push((vk, *pressed));
                                        }
                                    }
                                }
                                egui::Event::Text(t) => texts.push(t.clone()),
                                egui::Event::Copy => specials.push(1),
                                egui::Event::Cut => specials.push(2),
                                egui::Event::Paste(t) => {
                                    paste_text = Some(t.clone());
                                    specials.push(3);
                                }
                                _ => {}
                            }
                        }
                    });
                    for (vk, down) in keys {
                        locked.push(InputEvent::Key { vk, down });
                    }
                    for t in texts {
                        for ch in t.chars() {
                            if let Some((vk, sh, ct, al)) = input::vk_for_char(ch) {
                                if al {
                                    locked.push(InputEvent::Key {
                                        vk: 0x12,
                                        down: true,
                                    });
                                }
                                if ct {
                                    locked.push(InputEvent::Key {
                                        vk: 0x11,
                                        down: true,
                                    });
                                }
                                if sh {
                                    locked.push(InputEvent::Key {
                                        vk: 0x10,
                                        down: true,
                                    });
                                }
                                locked.push(InputEvent::Key { vk, down: true });
                                locked.push(InputEvent::Key { vk, down: false });
                                if sh {
                                    locked.push(InputEvent::Key {
                                        vk: 0x10,
                                        down: false,
                                    });
                                }
                                if ct {
                                    locked.push(InputEvent::Key {
                                        vk: 0x11,
                                        down: false,
                                    });
                                }
                                if al {
                                    locked.push(InputEvent::Key {
                                        vk: 0x12,
                                        down: false,
                                    });
                                }
                            }
                        }
                    }
                    // egui перехватывает Ctrl+C/X/V — транслируем на удалённую машину.
                    for s in specials {
                        let vk = match s {
                            1 => 0x43u16,
                            2 => 0x58,
                            _ => 0x56,
                        };
                        locked.push(InputEvent::Key { vk, down: true });
                        locked.push(InputEvent::Key { vk, down: false });
                    }
                    // Вставка: текст сначала отправим агенту (он положит его
                    // в свой буфер), клавиши Ctrl+V уже в очереди ввода.
                    if let Some(text) = paste_text
                        && !text.is_empty()
                    {
                        *v.pending_paste.lock().unwrap() = Some(text);
                    }
                    let m = ui.ctx().input(|i| i.modifiers);
                    let pm = v.prev_mods;
                    for (now_on, was_on, vk) in [
                        (m.shift, pm.shift, 0x10u16),
                        (m.ctrl, pm.ctrl, 0x11),
                        (m.alt, pm.alt, 0x12),
                        (m.mac_cmd, pm.mac_cmd, 0x5B),
                    ] {
                        if now_on && !was_on {
                            locked.push(InputEvent::Key { vk, down: true });
                        }
                        if !now_on && was_on {
                            locked.push(InputEvent::Key { vk, down: false });
                        }
                    }
                    v.prev_mods = m;
                }

                // Потеря фокуса: отпустить зажатое, чтобы не «залипло».
                if !focused {
                    if v.buttons_down.iter().any(|d| *d) {
                        for (idx, d) in v.buttons_down.iter_mut().enumerate() {
                            if *d {
                                *d = false;
                                let (x, y) = v.last_pointer.unwrap_or((32768, 32768));
                                locked.push(InputEvent::Button {
                                    btn: idx as u8,
                                    down: false,
                                    x,
                                    y,
                                });
                            }
                        }
                    }
                    let pm = v.prev_mods;
                    for (was_on, vk) in [
                        (pm.shift, 0x10u16),
                        (pm.ctrl, 0x11),
                        (pm.alt, 0x12),
                        (pm.mac_cmd, 0x5B),
                    ] {
                        if was_on {
                            locked.push(InputEvent::Key { vk, down: false });
                        }
                    }
                    v.prev_mods = egui::Modifiers::default();
                }
                drop(locked);

                // Удалённый курсор поверх видео.
                let cs = *cursor_arc.lock().unwrap();
                if cs.visible {
                    let (px, py) = input::norm_to_ui_px(cs.x, cs.y, draw.width(), draw.height());
                    let pos = egui::pos2(draw.min.x + px, draw.min.y + py);
                    ui.painter()
                        .circle_filled(pos, 5.0, egui::Color32::from_black_alpha(180));
                    ui.painter().circle_filled(pos, 2.5, egui::Color32::WHITE);
                }
            }

            if back {
                self.close_viewer();
            }
        }
    }

    /// VK печатных клавиш: их доставляет Text-событие, по Key-пути не дублируем.
    fn is_char_vk(vk: u16) -> bool {
        matches!(vk, 0x20 | 0x30..=0x39 | 0x41..=0x5A)
    }

    /// egui-клавиша → Windows VK (служебные клавиши; печатные идут через vk_for_char).
    fn vk_of_key(key: egui::Key) -> Option<u16> {
        use egui::Key as K;
        Some(match key {
            K::ArrowUp => 0x26,
            K::ArrowDown => 0x28,
            K::ArrowLeft => 0x25,
            K::ArrowRight => 0x27,
            K::Enter => 0x0D,
            K::Escape => 0x1B,
            K::Backspace => 0x08,
            K::Delete => 0x2E,
            K::Insert => 0x2D,
            K::Home => 0x24,
            K::End => 0x23,
            K::PageUp => 0x21,
            K::PageDown => 0x22,
            K::Tab => 0x09,
            K::Space => 0x20,
            K::F1 => 0x70,
            K::F2 => 0x71,
            K::F3 => 0x72,
            K::F4 => 0x73,
            K::F5 => 0x74,
            K::F6 => 0x75,
            K::F7 => 0x76,
            K::F8 => 0x77,
            K::F9 => 0x78,
            K::F10 => 0x79,
            K::F11 => 0x7A,
            K::F12 => 0x7B,
            K::A => 0x41,
            K::B => 0x42,
            K::C => 0x43,
            K::D => 0x44,
            K::E => 0x45,
            K::F => 0x46,
            K::G => 0x47,
            K::H => 0x48,
            K::I => 0x49,
            K::J => 0x4A,
            K::K => 0x4B,
            K::L => 0x4C,
            K::M => 0x4D,
            K::N => 0x4E,
            K::O => 0x4F,
            K::P => 0x50,
            K::Q => 0x51,
            K::R => 0x52,
            K::S => 0x53,
            K::T => 0x54,
            K::U => 0x55,
            K::V => 0x56,
            K::W => 0x57,
            K::X => 0x58,
            K::Y => 0x59,
            K::Z => 0x5A,
            K::Num0 => 0x30,
            K::Num1 => 0x31,
            K::Num2 => 0x32,
            K::Num3 => 0x33,
            K::Num4 => 0x34,
            K::Num5 => 0x35,
            K::Num6 => 0x36,
            K::Num7 => 0x37,
            K::Num8 => 0x38,
            K::Num9 => 0x39,
            _ => return None,
        })
    }

    impl RatApp {
        /// Операция со службой из интерфейса: сразу или через запрос UAC.
        #[cfg(feature = "ffmpeg")]
        fn svc_action(&mut self, op: crate::service::Op) {
            let (flag, name): (&str, &str) = match op {
                crate::service::Op::Install => ("--install-service", "Установка службы"),
                crate::service::Op::Uninstall => ("--uninstall-service", "Удаление службы"),
                crate::service::Op::Start => ("--start-service", "Запуск службы"),
                crate::service::Op::Stop => ("--stop-service", "Остановка службы"),
            };
            let result = match op {
                crate::service::Op::Install => crate::service::install(),
                crate::service::Op::Uninstall => crate::service::uninstall(),
                crate::service::Op::Start => crate::service::start(),
                crate::service::Op::Stop => crate::service::stop(),
            };
            match result {
                Ok(()) => {
                    self.svc_state = crate::service::status();
                    self.svc_checked = Instant::now();
                    self.toast(format!("{name}: выполнено"));
                }
                Err(e) => {
                    if e.contains("ошибка 5") {
                        match crate::elevate::relaunch(flag) {
                            Ok(()) => {
                                self.svc_state = crate::service::State::Pending;
                                self.svc_checked = Instant::now();
                                self.toast(format!(
                                    "{name}: подтвердите запрос прав администратора"
                                ));
                            }
                            Err(e2) => self.toast(format!("{name}: {e2}")),
                        }
                    } else {
                        self.toast(format!("{name}: {e}"));
                    }
                }
            }
        }

        /// Отключиться от удалённого ПК и вернуться к списку машин.
        fn close_viewer(&mut self) {
            if let Some(mut s) = self.viewer.take() {
                s.shutdown();
            }
            self.screen = Screen::Home;
        }

        /// Крестик окна прячет приложение в трей, сессии продолжают работать.
        fn handle_close(&mut self, ctx: &egui::Context) {
            if self.quitting {
                return;
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                // Крестик при активной сессии = отключиться и вернуться к
                // списку машин (окно остаётся; повторный крестик — как обычно).
                if self.viewer.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    if ctx.input(|i| i.viewport().fullscreen).unwrap_or(false) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                    }
                    self.close_viewer();
                    return;
                }
                // В режиме агента (--tray) закрытие окна только прячет его: агент
                // продолжает работать, иначе служба перезапускала бы его по кругу,
                // а активная сессия оператора рвалась бы.
                if self.tray.is_none() && !self.args.tray {
                    return;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.hidden = true;
            }
        }

        /// Команды меню значка: «Показать окно» и «Выход».
        fn poll_tray(&mut self, ctx: &egui::Context) {
            let Some(cmd) = self.tray.as_ref().and_then(|t| t.poll()) else {
                return;
            };
            match cmd {
                crate::tray::Cmd::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.hidden = false;
                }
                crate::tray::Cmd::Quit => {
                    self.quitting = true;
                    self.internet.stop();
                    self.server.stop();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    impl eframe::App for RatApp {
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            let ctx = ui.ctx().clone();
            // Первый кадр: запомнить контекст egui — поток слушателя
            // повторного запуска будит им перерисовку.
            if let Ok(mut slot) = self.repaint_slot.lock()
                && slot.is_none()
            {
                *slot = Some(ctx.clone());
            }
            // Повторный запуск: показать окно (как клик по значку в трее).
            if self.show_request.as_ref().is_some_and(|s| s.take()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                self.hidden = false;
            }
            self.poll_tray(&ctx);
            self.handle_close(&ctx);
            if !self.tray_hide_done {
                self.tray_hide_done = true;
                // Режим агента (--tray, служба): окно стартует скрытым даже без
                // значка трея — на управляемой машине окно не должно висеть.
                if self.args.tray {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    self.hidden = true;
                }
            }
            if !self.autoconnect_done {
                self.autoconnect_done = true;
                if let Some(raw_addr) = self.args.autoconnect.clone() {
                    let addr = match self.device.mode {
                        ConnectionMode::Internet => raw_addr.trim().to_string(),
                        ConnectionMode::Lan => normalize_addr(&raw_addr),
                    };
                    if self.device.mode == ConnectionMode::Internet && !is_valid_id(&addr) {
                        self.toast("Для интернет-подключения нужен ID из 6 цифр");
                    } else {
                        self.start_connect(addr.clone(), addr);
                    }
                }
            }
            self.poll_connect();
            match self.screen {
                Screen::Home => self.home_ui(ui),
                Screen::Viewer => self.viewer_ui(ui),
            }
            self.dialogs(ui.ctx());
            if let Some(secs) = self.args.smoke
                && self.started.elapsed().as_secs_f32() >= secs
                && !self.finished
            {
                self.finished = true;
                if let Some(v) = self.viewer.as_mut() {
                    v.shutdown();
                }
                self.server.stop();
                crate::note("APP_DONE");
            }
            if self.finished {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if self.hidden {
                // Окно в трее: сессии продолжают работать, UI просыпается редко.
                ctx.request_repaint_after(Duration::from_millis(250));
            } else {
                ctx.request_repaint_after(Duration::from_millis(10));
            }
        }
    }
}
