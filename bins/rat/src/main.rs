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
}

#[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
fn parse_args() -> Args {
    let mut a = Args {
        port: 55140,
        monitor: 0,
        autoconnect: None,
        smoke: None,
        switch_after: None,
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
    if let Err(e) = ui::run(args) {
        note(&format!("rat: {e}"));
        std::process::exit(1);
    }
}

// ================== журнал ==================

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

    /// Нормализовать адрес: добавить порт по умолчанию.
    pub fn normalize_addr(raw: &str) -> String {
        let s = raw.trim();
        if s.contains(':') {
            s.to_string()
        } else {
            format!("{s}:55140")
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
}

// ================== встроенный сервер ==================

#[cfg(feature = "ffmpeg")]
mod server {
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use rat_session::agent::{AgentConfig, spawn_agent};
    use rat_session::link::tcp_accept;
    use rat_session::stats::{SessionStats, TimeBase};

    pub struct ServerHandle {
        pub port: u16,
        running: Arc<AtomicBool>,
        stop_current: Arc<Mutex<Option<Arc<AtomicBool>>>>,
        thread: Option<JoinHandle<()>>,
        pub sessions: Arc<AtomicU64>,
        /// Какой монитор отдавать (индекс DXGI; применяется к новым сессиям).
        pub monitor: Arc<AtomicU32>,
        pub last_err: Arc<Mutex<Option<String>>>,
    }

    impl ServerHandle {
        pub fn new() -> Self {
            Self {
                port: 55140,
                running: Arc::new(AtomicBool::new(false)),
                stop_current: Arc::new(Mutex::new(None)),
                thread: None,
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
            let sessions = self.sessions.clone();
            let monitor = self.monitor.clone();
            let last_err = self.last_err.clone();
            crate::note(&format!("[server] слушаю 0.0.0.0:{port}"));
            self.thread = Some(std::thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    match tcp_accept(&listener) {
                        Ok((link, closer)) => {
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

    impl Default for ServerHandle {
        fn default() -> Self {
            Self::new()
        }
    }
}
// ================== приложение (egui) ==================

#[cfg(feature = "ffmpeg")]
mod ui {
    use std::collections::HashMap;
    use std::net::ToSocketAddrs;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use eframe::egui;
    use rat_session::capture::MonitorInfo;
    use rat_session::input::{self, CursorState, InputEvent, SessionInfo};
    use rat_session::link::{TcpCloser, tcp_connect};
    use rat_session::stats::{SessionStats, TimeBase};
    use rat_session::viewer::{FrameOut, ViewerShared, spawn_viewer};

    use super::Args;
    use super::server::ServerHandle;
    use super::store::{Machine, ago_text, load, normalize_addr, now_unix, save};

    const CARD_BG: egui::Color32 = egui::Color32::from_rgb(34, 39, 47);
    const CARD_BORDER: egui::Color32 = egui::Color32::from_rgb(70, 76, 88);
    const CARD_BORDER_HOVER: egui::Color32 = egui::Color32::from_rgb(64, 140, 255);
    const ON: egui::Color32 = egui::Color32::from_rgb(76, 175, 80);
    const OFF: egui::Color32 = egui::Color32::from_rgb(244, 67, 54);
    const DIM: egui::Color32 = egui::Color32::from_rgb(150, 158, 170);
    const DIM2: egui::Color32 = egui::Color32::from_rgb(120, 128, 140);

    pub fn run(args: Args) -> Result<(), String> {
        if std::env::var_os("WGPU_BACKEND").is_none() {
            // SAFETY: до инициализации GUI, однопоточно.
            unsafe { std::env::set_var("WGPU_BACKEND", "dx12") };
        }
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1020.0, 700.0]),
            ..Default::default()
        };
        eframe::run_native(
            "Rat Next",
            options,
            Box::new(move |_cc| Ok(Box::new(RatApp::new(args)))),
        )
        .map_err(|e| e.to_string())
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
    }

    impl RatApp {
        fn new(args: Args) -> Self {
            let machines = Arc::new(Mutex::new(load()));
            let statuses: Arc<Mutex<HashMap<String, bool>>> = Arc::new(Mutex::new(HashMap::new()));
            {
                let machines = machines.clone();
                let statuses = statuses.clone();
                std::thread::spawn(move || {
                    loop {
                        let list: Vec<String> = machines
                            .lock()
                            .unwrap()
                            .iter()
                            .map(|m| m.addr.clone())
                            .collect();
                        for addr in list {
                            let ok = addr
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
                                .unwrap_or(false);
                            statuses.lock().unwrap().insert(addr, ok);
                        }
                        std::thread::sleep(Duration::from_secs(4));
                    }
                });
            }
            let port = args.port;
            let mut server = ServerHandle::new();
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
            Self {
                machines,
                statuses,
                server,
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
            std::thread::spawn(move || {
                let mut connected = None;
                let mut last_err = String::new();
                for attempt in 0..10 {
                    match tcp_connect(addr_conn.as_str()) {
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
                    ui.label(if running {
                        "принимаю подключения"
                    } else {
                        "сервер остановлен"
                    });
                    ui.separator();
                    if self.my_name.is_empty() {
                        ui.label(format!(
                            "Этот компьютер: {}:{}",
                            self.my_ip, self.server.port
                        ));
                    } else {
                        ui.label(format!(
                            "Этот компьютер: {} ({}:{})",
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
                    if running {
                        let stop_clicked = ui.button("Остановить сервер").clicked();
                        if stop_clicked {
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
                    ui.label(format!(
                        "сессий: {}",
                        self.server.sessions.load(Ordering::Relaxed)
                    ));
                    if let Some(e) = self.server.last_err.lock().unwrap().clone() {
                        ui.separator();
                        ui.colored_label(OFF, format!("ошибка: {e}"));
                    }
                });
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
                        ui.label("Адрес (IP:порт):");
                        ui.add(egui::TextEdit::singleline(&mut self.add_addr).desired_width(280.0));
                        if let Some(e) = &self.add_err {
                            ui.colored_label(OFF, e);
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Добавить").clicked() {
                                let addr = normalize_addr(&self.add_addr);
                                let (host, port) = split_addr(&addr);
                                if host.is_empty() || port.is_none() {
                                    self.add_err = Some(
                                        "Адрес должен быть в виде IP:порт, напр. 192.168.1.50:55140"
                                            .into(),
                                    );
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
            egui::Panel::top("vt").show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("← Назад").clicked() {
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
                        ui.label("Монитор:");
                        for (idx, mw, mh) in &mons {
                            let sel = cur == *idx;
                            if ui
                                .selectable_label(sel, format!("{} ({}×{})", idx + 1, mw, mh))
                                .clicked()
                                && !sel
                            {
                                *v.pending_switch.lock().unwrap() = Some(*idx);
                            }
                        }
                    } else if let Some(si) = v.info.lock().unwrap().as_ref() {
                        ui.label(format!("монитор {}×{}", si.out.width, si.out.height));
                    }
                });
            });
            egui::Panel::bottom("vh").show(ui, |ui| {
                ui.label(
                    "Мышь — над видео; клавиатура — когда окно в фокусе. «← Назад» — отключиться.",
                );
            });

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

                // Указатель: движение, кнопки, колесо.
                let engaged = resp.hovered() || resp.dragged() || v.buttons_down.iter().any(|d| *d);
                if engaged {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                    if let Some(p) = ui.ctx().input(|i| i.pointer.latest_pos()) {
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
                        if resp.hovered() || v.buttons_down[i] {
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
                    if resp.hovered() && (wheel.0.abs() > 0.5 || wheel.1.abs() > 0.5) {
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
                                        let shortcut = modifiers.ctrl || modifiers.alt;
                                        if shortcut || !is_char_vk(vk) {
                                            keys.push((vk, *pressed));
                                        }
                                    }
                                }
                                egui::Event::Text(t) => texts.push(t.clone()),
                                egui::Event::Copy => specials.push(1),
                                egui::Event::Cut => specials.push(2),
                                egui::Event::Paste(_) => specials.push(3),
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
                    let m = ui.ctx().input(|i| i.modifiers);
                    let pm = v.prev_mods;
                    for (now_on, was_on, vk) in [
                        (m.shift, pm.shift, 0x10u16),
                        (m.ctrl, pm.ctrl, 0x11),
                        (m.alt, pm.alt, 0x12),
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
                    for (was_on, vk) in [(pm.shift, 0x10u16), (pm.ctrl, 0x11), (pm.alt, 0x12)] {
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
                if let Some(mut s) = self.viewer.take() {
                    s.shutdown();
                }
                self.screen = Screen::Home;
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

    impl eframe::App for RatApp {
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            if !self.autoconnect_done {
                self.autoconnect_done = true;
                if let Some(a) = self.args.autoconnect.clone() {
                    let a = normalize_addr(&a);
                    self.start_connect(a.clone(), a);
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
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(10));
            }
        }
    }
}
