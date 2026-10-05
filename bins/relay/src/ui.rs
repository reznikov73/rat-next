//! Главное окно relay: журнал, управление запуском, монитор подключений,
//! окно настроек и работа с занятым портом.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::core::{Event, Handle};
use crate::port::{self, Owner};
use crate::settings::{self, Settings};
use crate::tray::{Cmd, Tray};

const ON: egui::Color32 = egui::Color32::from_rgb(76, 175, 80);
const OFF: egui::Color32 = egui::Color32::from_rgb(244, 67, 54);
const DIM: egui::Color32 = egui::Color32::from_rgb(150, 158, 170);
const LOG_LINES: usize = 4000;

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Log,
    Clients,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClientState {
    Waiting,
    Active,
    Done,
}

struct Client {
    id: String,
    name: String,
    state: ClientState,
    since: Instant,
    sessions: u32,
}

/// Журнал: хранит последние строки и собирает текст для поля вывода.
struct LogBuf {
    lines: VecDeque<String>,
    text: String,
    dirty: bool,
}

impl LogBuf {
    fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            text: String::new(),
            dirty: false,
        }
    }

    fn push(&mut self, line: String) {
        self.lines.push_back(line);
        while self.lines.len() > LOG_LINES {
            self.lines.pop_front();
        }
        self.dirty = true;
    }

    fn text(&mut self) -> &str {
        if self.dirty {
            self.text.clear();
            for line in &self.lines {
                self.text.push_str(line);
                self.text.push('\n');
            }
            self.dirty = false;
        }
        &self.text
    }

    fn full(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.text.clear();
        self.dirty = false;
    }
}

pub struct RelayApp {
    settings: Settings,
    settings_open: bool,
    edit_public: String,
    edit_listen: String,
    settings_error: Option<String>,
    restart_after_stop: bool,
    handle: Option<Handle>,
    stopping: Option<Handle>,
    log: LogBuf,
    follow: bool,
    clients: Vec<Client>,
    tab: Tab,
    tray: Option<Tray>,
    port_owners: Vec<Owner>,
    port_window: bool,
    port_error: Option<String>,
    quitting: bool,
    start_hidden: bool,
    hidden_once: bool,
    autostart_checked: bool,
    started_at: Option<Instant>,
    relay_starts: u32,
}

pub fn run(initial_listen: Option<String>, start_hidden: bool) -> Result<(), String> {
    if std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: до инициализации GUI, однопоточно.
        unsafe { std::env::set_var("WGPU_BACKEND", "dx12") };
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rat Next relay")
            .with_icon(window_icon())
            .with_inner_size([900.0, 620.0])
            .with_min_inner_size([680.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Rat Next relay",
        options,
        Box::new(move |_cc| Ok(Box::new(RelayApp::new(initial_listen, start_hidden)))),
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

impl RelayApp {
    fn new(initial_listen: Option<String>, start_hidden: bool) -> Self {
        let mut settings = settings::load();
        if let Some(listen) = initial_listen {
            settings.listen_addr = listen;
        }
        let mut app = Self {
            settings_open: false,
            edit_public: settings.public_addr.clone(),
            edit_listen: settings.listen_addr.clone(),
            settings,
            settings_error: None,
            restart_after_stop: false,
            handle: None,
            stopping: None,
            log: LogBuf::new(),
            follow: true,
            clients: Vec::new(),
            tab: Tab::Log,
            tray: None,
            port_owners: Vec::new(),
            port_window: false,
            port_error: None,
            quitting: false,
            start_hidden,
            hidden_once: false,
            autostart_checked: false,
            started_at: None,
            relay_starts: 0,
        };
        app.log_line("Rat Next relay запущен");
        app.log_line(format!("адрес для клиентов: {}", app.settings.public_addr));
        match Tray::new(&app.settings.listen_addr, false) {
            Ok(tray) => app.tray = Some(tray),
            Err(e) => app.log_line(format!("трей недоступен: {e}")),
        }
        app.start();
        app
    }

    fn running(&self) -> bool {
        self.handle.is_some()
    }

    fn log_line(&mut self, line: impl Into<String>) {
        let stamp = time_text();
        self.log.push(format!("{stamp} {}", line.into()));
    }

    fn start(&mut self) {
        if self.handle.is_some() || self.stopping.is_some() {
            return;
        }
        let listen = self.settings.listen_addr.clone();
        match Handle::start(&listen) {
            Ok(handle) => {
                self.relay_starts += 1;
                self.started_at = Some(Instant::now());
                self.port_owners.clear();
                self.port_error = None;
                self.log_line(format!("relay слушает {listen}"));
                self.handle = Some(handle);
            }
            Err(e) => {
                self.log_line(format!("не удалось запустить relay: {e}"));
                if let Some(p) = settings::port_of(&listen) {
                    let owners = port::owners(p);
                    if !owners.is_empty() {
                        let names = owners
                            .iter()
                            .map(|o| o.label())
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.log_line(format!("порт {p} занят: {names}"));
                        self.port_owners = owners;
                        self.port_window = true;
                    }
                }
            }
        }
        self.sync_tray();
    }

    fn stop(&mut self, restart: bool) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        handle.request_stop();
        self.stopping = Some(handle);
        self.restart_after_stop = restart;
        self.started_at = None;
        self.log_line("остановка relay…");
        self.sync_tray();
    }

    fn poll_stopping(&mut self) {
        let finished = self
            .stopping
            .as_ref()
            .map(|h| h.is_finished())
            .unwrap_or(false);
        if !finished {
            return;
        }
        self.stopping = None;
        self.log_line("relay остановлен, порт освобождён");
        if self.restart_after_stop {
            self.restart_after_stop = false;
            self.start();
        }
    }

    fn sync_tray(&mut self) {
        if let Some(tray) = &self.tray {
            tray.set_state(&self.settings.listen_addr, self.running());
        }
    }

    fn pump_events(&mut self) {
        let mut batch = Vec::new();
        if let Some(handle) = &self.handle {
            while let Ok(event) = handle.events.try_recv() {
                batch.push(event);
            }
        }
        for event in batch {
            match event {
                Event::Log(line) => self.log_line(line),
                Event::Registered { id, name } => {
                    let label = if name.trim().is_empty() {
                        "без имени".to_string()
                    } else {
                        name
                    };
                    self.upsert(&id, label, ClientState::Waiting);
                }
                Event::SessionStart { id } => self.upsert(&id, String::new(), ClientState::Active),
                Event::SessionEnd { id } => self.upsert(&id, String::new(), ClientState::Done),
            }
        }
    }

    fn upsert(&mut self, id: &str, name: String, state: ClientState) {
        if let Some(client) = self.clients.iter_mut().find(|c| c.id == id) {
            if !name.is_empty() {
                client.name = name;
            }
            if state == ClientState::Active && client.state != ClientState::Active {
                client.sessions += 1;
            }
            client.state = state;
            client.since = Instant::now();
            return;
        }
        self.clients.push(Client {
            id: id.to_string(),
            name: if name.is_empty() {
                "без имени".to_string()
            } else {
                name
            },
            state,
            since: Instant::now(),
            sessions: u32::from(state == ClientState::Active),
        });
    }

    fn open_settings(&mut self) {
        self.edit_public = self.settings.public_addr.clone();
        self.edit_listen = self.settings.listen_addr.clone();
        self.settings_error = None;
        self.autostart_checked = crate::autostart::is_installed();
        self.settings_open = true;
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        if let Some(handle) = self.handle.take() {
            handle.request_stop();
        }
        if let Some(handle) = self.stopping.take() {
            handle.request_stop();
        }
        self.log_line("выход");
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

fn hide_window(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
}

fn show_window(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

fn time_text() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = secs % 86400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

fn ago(from: Instant) -> String {
    let secs = from.elapsed().as_secs();
    if secs < 60 {
        format!("{secs} с назад")
    } else if secs < 3600 {
        format!("{} мин назад", secs / 60)
    } else {
        format!("{} ч назад", secs / 3600)
    }
}

impl eframe::App for RelayApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if self.start_hidden && !self.hidden_once {
            self.hidden_once = true;
            hide_window(&ctx);
        }

        self.pump_events();
        self.poll_stopping();
        self.poll_tray(&ctx);
        self.handle_close(&ctx);

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(6.0);
            self.toolbar(ui);
            ui.add_space(6.0);
        });
        egui::Panel::top("tabs").show(ui, |ui| {
            ui.add_space(4.0);
            self.tabs(ui);
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Log => self.log_view(ui),
            Tab::Clients => self.clients_view(ui),
        });

        self.settings_window(&ctx);
        self.port_window(&ctx);

        // События relay и трея подхватываются и при скрытом окне.
        if !self.quitting {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

impl RelayApp {
    /// Крестик окна прячет приложение в трей, relay продолжает работать.
    fn handle_close(&mut self, ctx: &egui::Context) {
        if self.quitting {
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            hide_window(ctx);
        }
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        let Some(cmd) = self.tray.as_ref().and_then(|t| t.poll()) else {
            return;
        };
        match cmd {
            Cmd::Show => show_window(ctx),
            Cmd::Toggle => {
                if self.running() {
                    self.stop(false);
                } else {
                    self.start();
                }
            }
            Cmd::Settings => {
                self.open_settings();
                show_window(ctx);
            }
            Cmd::Quit => self.quit(ctx),
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (mark, state, color) = if self.running() {
                ("●", "работает", ON)
            } else if self.stopping.is_some() {
                ("●", "останавливается…", DIM)
            } else {
                ("○", "остановлен", OFF)
            };
            ui.colored_label(color, mark);
            ui.colored_label(color, state);
            ui.separator();
            ui.label(format!("Слушает: {}", self.settings.listen_addr));
            ui.separator();
            ui.label(format!("Для клиентов: {}", self.settings.public_addr));
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let busy = self.stopping.is_some();
            if self.running() {
                if ui
                    .add_enabled(!busy, egui::Button::new("Остановить relay"))
                    .clicked()
                {
                    self.stop(false);
                }
            } else if ui
                .add_enabled(!busy, egui::Button::new("Запустить relay"))
                .clicked()
            {
                self.start();
            }
            if ui.button("Настройки…").clicked() {
                self.open_settings();
            }
            if ui.button("Освободить порт…").clicked() {
                self.refresh_port_owners();
            }
            if ui.button("Свернуть в трей").clicked() {
                hide_window(ui.ctx());
            }
            if let Some(started) = self.started_at {
                ui.colored_label(DIM, format!("запущен {}", ago(started)));
            }
        });
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        let active = self
            .clients
            .iter()
            .filter(|c| c.state == ClientState::Active)
            .count();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Log, "Логи");
            ui.selectable_value(
                &mut self.tab,
                Tab::Clients,
                format!("Подключения ({})", self.clients.len()),
            );
            ui.separator();
            ui.colored_label(
                if active > 0 { ON } else { DIM },
                format!("активных сессий: {active}"),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Копировать лог").clicked() {
                    ui.ctx().copy_text(self.log.full());
                }
            });
        });
    }

    fn log_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Копировать всё").clicked() {
                ui.ctx().copy_text(self.log.full());
            }
            if ui.button("Очистить").clicked() {
                self.log.clear();
            }
            ui.checkbox(&mut self.follow, "Следовать за концом");
            if ui.button("К концу").clicked() {
                self.follow = true;
            }
            ui.colored_label(DIM, "текст можно выделять и копировать");
        });
        ui.add_space(4.0);

        // Прокрутка вверх останавливает автопрокрутку, чтобы не мешать чтению.
        if ui.input(|i| i.smooth_scroll_delta.y) > 0.0 {
            self.follow = false;
        }
        let follow = self.follow;
        let mut view: &str = self.log.text();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(follow)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut view)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(26),
                );
            });
    }
}

impl RelayApp {
    fn clients_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Очистить список").clicked() {
                self.clients.clear();
            }
            ui.colored_label(DIM, "список обновляется событиями relay");
        });
        ui.add_space(4.0);
        if self.clients.is_empty() {
            ui.colored_label(DIM, "пока никто не регистрировался");
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("clients")
                    .num_columns(5)
                    .striped(true)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.strong("ID");
                        ui.strong("Имя");
                        ui.strong("Состояние");
                        ui.strong("Сессий");
                        ui.strong("Обновлено");
                        ui.end_row();
                        for client in &self.clients {
                            ui.label(&client.id);
                            ui.label(&client.name);
                            let (text, color) = match client.state {
                                ClientState::Waiting => ("ожидает", DIM),
                                ClientState::Active => ("активная сессия", ON),
                                ClientState::Done => ("завершена", DIM),
                            };
                            ui.colored_label(color, text);
                            ui.label(client.sessions.to_string());
                            ui.label(ago(client.since));
                            ui.end_row();
                        }
                    });
            });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let mut save = false;
        let mut defaults = false;
        let mut close = false;
        egui::Window::new("Настройки relay")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Адрес для клиентов (его вводят в Rat Next):");
                ui.add(egui::TextEdit::singleline(&mut self.edit_public).desired_width(320.0));
                ui.colored_label(DIM, format!("по умолчанию {}", settings::DEFAULT_PUBLIC));
                ui.add_space(8.0);
                ui.label("Адрес прослушивания:");
                ui.add(egui::TextEdit::singleline(&mut self.edit_listen).desired_width(320.0));
                ui.colored_label(DIM, format!("по умолчанию {}", settings::DEFAULT_LISTEN));
                ui.add_space(10.0);
                let mut autostart = self.autostart_checked;
                if ui
                    .checkbox(
                        &mut autostart,
                        "Запускать при входе в Windows (сразу в трей)",
                    )
                    .changed()
                {
                    let res = if autostart {
                        crate::autostart::install()
                    } else {
                        crate::autostart::remove()
                    };
                    match res {
                        Ok(()) => {
                            self.autostart_checked = autostart;
                            self.settings_error = None;
                        }
                        Err(e) => self.settings_error = Some(format!("автозапуск: {e}")),
                    }
                }
                ui.add_space(10.0);
                if let Some(err) = &self.settings_error {
                    ui.colored_label(OFF, err);
                    ui.add_space(6.0);
                }
                ui.horizontal(|ui| {
                    save = ui.button("Сохранить").clicked();
                    defaults = ui.button("Значения по умолчанию").clicked();
                    close = ui.button("Закрыть").clicked();
                });
            });
        if !open || close {
            self.settings_open = false;
        }
        if defaults {
            self.edit_public = settings::DEFAULT_PUBLIC.to_string();
            self.edit_listen = settings::DEFAULT_LISTEN.to_string();
            self.settings_error = None;
        }
        if save {
            self.save_settings();
        }
    }

    fn save_settings(&mut self) {
        let public = match settings::validate_public(&self.edit_public) {
            Ok(v) => v,
            Err(e) => {
                self.settings_error = Some(e);
                return;
            }
        };
        let listen = match settings::validate_listen(&self.edit_listen) {
            Ok(v) => v,
            Err(e) => {
                self.settings_error = Some(e);
                return;
            }
        };
        let listen_changed = listen != self.settings.listen_addr;
        self.settings.public_addr = public;
        self.settings.listen_addr = listen;
        self.settings_error = None;
        self.settings_open = false;
        match settings::save(&self.settings) {
            Ok(()) => self.log_line("настройки сохранены"),
            Err(e) => self.log_line(format!("не удалось сохранить настройки: {e}")),
        }
        if listen_changed {
            let listen = self.settings.listen_addr.clone();
            self.log_line(format!("адрес прослушивания изменён: {listen}"));
            if self.running() {
                self.stop(true);
            } else {
                self.start();
            }
        }
        self.sync_tray();
    }

    fn refresh_port_owners(&mut self) {
        let Some(port) = settings::port_of(&self.settings.listen_addr) else {
            self.log_line("не удалось определить порт из адреса прослушивания");
            return;
        };
        let owners = port::owners(port);
        if owners.is_empty() {
            self.port_owners.clear();
            self.port_error = None;
            self.log_line(format!("порт {port} свободен"));
        } else {
            let names = owners
                .iter()
                .map(|o| o.label())
                .collect::<Vec<_>>()
                .join(", ");
            self.log_line(format!("порт {port} занят: {names}"));
            self.port_owners = owners;
            self.port_window = true;
        }
    }

    fn port_window(&mut self, ctx: &egui::Context) {
        if !self.port_window {
            return;
        }
        let mut open = true;
        let mut kill: Option<u32> = None;
        let mut kill_all = false;
        let mut refresh = false;
        let mut close = false;
        egui::Window::new("Освобождение порта")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Адрес прослушивания: {}",
                    self.settings.listen_addr
                ));
                ui.add_space(6.0);
                if self.port_owners.is_empty() {
                    ui.colored_label(DIM, "процессы не найдены — порт свободен");
                }
                let owners = &self.port_owners;
                for owner in owners {
                    ui.horizontal(|ui| {
                        ui.label(owner.label());
                        if ui.button("Завершить").clicked() {
                            kill = Some(owner.pid);
                        }
                    });
                }
                if let Some(err) = &self.port_error {
                    ui.add_space(6.0);
                    ui.colored_label(OFF, err);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    refresh = ui.button("Обновить").clicked();
                    kill_all = ui.button("Завершить все и запустить relay").clicked();
                    close = ui.button("Закрыть").clicked();
                });
            });
        if !open || close {
            self.port_window = false;
        }
        if refresh {
            self.refresh_port_owners();
        }
        if let Some(pid) = kill {
            match port::kill(pid) {
                Ok(()) => {
                    self.log_line(format!("процесс PID {pid} завершён"));
                    self.port_error = None;
                }
                Err(e) => self.port_error = Some(e),
            }
            self.refresh_port_owners();
        }
        if kill_all {
            let pids: Vec<u32> = self.port_owners.iter().map(|o| o.pid).collect();
            for pid in pids {
                match port::kill(pid) {
                    Ok(()) => self.log_line(format!("процесс PID {pid} завершён")),
                    Err(e) => self.port_error = Some(e),
                }
            }
            self.refresh_port_owners();
            if self.port_owners.is_empty() {
                self.port_window = false;
                self.start();
            }
        }
    }
}
