//! `rat-viewer` — вьюер оператора.
//!
//! P2 (пилот): TCP-подключение к агенту, приём mux-кадров → H.264 декод → показ (egui).
//!
//! Запуск:
//! ```text
//! cargo run -p rat-viewer --release --features ffmpeg -- --connect 127.0.0.1:55140
//! cargo run -p rat-viewer --release --features ffmpeg -- --connect 127.0.0.1:55140 --smoke=15
//! ```

#[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
struct Args {
    connect: String,
    smoke: Option<f32>,
}

fn parse_args() -> Args {
    let mut a = Args {
        connect: "127.0.0.1:55140".to_string(),
        smoke: None,
    };
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        let cur = args[i].as_str();
        match cur {
            "--connect" if i + 1 < args.len() => {
                a.connect = args[i + 1].clone();
                i += 2;
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
    eprintln!("rat-viewer: соберите с --features ffmpeg (пилотный кодек-путь P2)");
    std::process::exit(3);
}

#[cfg(feature = "ffmpeg")]
fn main() -> eframe::Result<()> {
    ui::run(parse_args())
}

#[cfg(feature = "ffmpeg")]
mod ui {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use eframe::egui;
    use rat_session::input::{self, CursorState, InputEvent, SessionInfo};
    use rat_session::link::{TcpCloser, tcp_connect};
    use rat_session::stats::{SessionStats, TimeBase};
    use rat_session::viewer::{FrameOut, ViewerShared, spawn_viewer};

    use super::Args;

    pub fn run(args: Args) -> eframe::Result<()> {
        if std::env::var_os("WGPU_BACKEND").is_none() {
            // SAFETY: до инициализации GUI, однопоточно.
            unsafe { std::env::set_var("WGPU_BACKEND", "dx12") };
        }
        println!("VIEWER_START connect={}", args.connect);
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 820.0]),
            ..Default::default()
        };
        let connect = args.connect.clone();
        eframe::run_native(
            "rat viewer — P2 pilot",
            options,
            Box::new(move |_cc| Ok(Box::new(App::new(connect, args.smoke)))),
        )?;
        println!("VIEWER_DONE");
        Ok(())
    }

    struct App {
        rx: Receiver<FrameOut>,
        stats: Arc<Mutex<SessionStats>>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
        closer: TcpCloser,
        base: TimeBase,
        texture: Option<egui::TextureHandle>,
        last: Option<FrameOut>,
        started: Instant,
        smoke: Option<f32>,
        finished: bool,
        ui_e2e_ms: Vec<f32>,
        connect: String,
        input_q: Arc<Mutex<Vec<InputEvent>>>,
        cursor: Arc<Mutex<CursorState>>,
        info: Arc<Mutex<Option<SessionInfo>>>,
        monitors: Arc<Mutex<Vec<(u16, u16, u16)>>>,
        pending_switch: Arc<Mutex<Option<u16>>>,
        last_pointer: Option<(u16, u16)>,
        prev_mods: egui::Modifiers,
        buttons_down: [bool; 5],
    }

    impl App {
        fn new(connect: String, smoke: Option<f32>) -> Self {
            // Подключение с повторами (~10 с).
            let mut connected = None;
            let mut last_err = String::new();
            for attempt in 0..20 {
                match tcp_connect(connect.as_str()) {
                    Ok(v) => {
                        connected = Some(v);
                        break;
                    }
                    Err(e) => {
                        last_err = e.to_string();
                        if attempt < 19 {
                            std::thread::sleep(Duration::from_millis(500));
                        }
                    }
                }
            }
            let Some((link, closer)) = connected else {
                eprintln!("не удалось подключиться к {connect}: {last_err}");
                std::process::exit(1);
            };

            let stats = Arc::new(Mutex::new(SessionStats::default()));
            let stop = Arc::new(AtomicBool::new(false));
            let base = TimeBase(Instant::now());
            let (tx, rx) = sync_channel::<FrameOut>(3);
            let input_q: Arc<Mutex<Vec<InputEvent>>> = Arc::new(Mutex::new(Vec::new()));
            let cursor = Arc::new(Mutex::new(CursorState::default()));
            let info: Arc<Mutex<Option<SessionInfo>>> = Arc::new(Mutex::new(None));
            let monitors: Arc<Mutex<Vec<(u16, u16, u16)>>> = Arc::new(Mutex::new(Vec::new()));
            let pending_switch: Arc<Mutex<Option<u16>>> = Arc::new(Mutex::new(None));
            let handle = spawn_viewer(
                base,
                stop.clone(),
                stats.clone(),
                link,
                tx,
                ViewerShared {
                    input_q: input_q.clone(),
                    cursor: cursor.clone(),
                    info: info.clone(),
                    monitors: monitors.clone(),
                    pending_switch: pending_switch.clone(),
                },
            );

            Self {
                rx,
                stats,
                stop,
                handle: Some(handle),
                closer,
                base,
                texture: None,
                last: None,
                started: Instant::now(),
                smoke,
                finished: false,
                ui_e2e_ms: Vec::new(),
                connect,
                input_q,
                cursor,
                info,
                monitors,
                pending_switch,
                last_pointer: None,
                prev_mods: egui::Modifiers::default(),
                buttons_down: [false; 5],
            }
        }

        fn finish(&mut self) {
            if self.finished {
                return;
            }
            self.finished = true;
            self.stop.store(true, Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
            self.closer.close();
            {
                let st = self.stats.lock().unwrap();
                println!("=== VIEWER SUMMARY ===");
                println!("{}", st.summary());
            }
            if !self.ui_e2e_ms.is_empty() {
                println!(
                    "ui present e2e: avg {:.2}ms p95 {:.2}ms",
                    SessionStats::avg(&self.ui_e2e_ms),
                    SessionStats::pct(&self.ui_e2e_ms, 0.95)
                );
            }
        }
    }

    impl eframe::App for App {
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            let input_q = self.input_q.clone();
            let cursor = self.cursor.clone();
            let info = self.info.clone();
            let monitors = self.monitors.clone();
            let pending_switch = self.pending_switch.clone();
            loop {
                match self.rx.try_recv() {
                    Ok(f) => {
                        let now = rat_session::stats::now_us(&self.base);
                        let e = ((now as i64 - f.t_capture_us as i64).max(0)) as f32 / 1000.0;
                        SessionStats::push_sample(&mut self.ui_e2e_ms, e);
                        self.last = Some(f);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break,
                }
            }

            if let Some(f) = &self.last {
                let img = egui::ColorImage::from_rgba_unmultiplied([f.w, f.h], &f.rgba);
                let data = egui::ImageData::Color(std::sync::Arc::new(img));
                match &mut self.texture {
                    Some(t) => t.set(data, egui::TextureOptions::LINEAR),
                    None => {
                        self.texture = Some(ui.ctx().load_texture(
                            "video",
                            data,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
            }

            egui::Panel::top("status").show(ui, |ui| {
                ui.horizontal(|ui| {
                    let st = self.stats.lock().unwrap();
                    ui.label(format!("connect: {}", self.connect));
                    ui.separator();
                    ui.label(format!("fps: {:.1}", st.fps));
                    ui.separator();
                    ui.label(format!(
                        "frame #{}",
                        self.last.as_ref().map(|f| f.id).unwrap_or(0)
                    ));
                    ui.separator();
                    ui.label(format!(
                        "sent {} | shown {} | drops(a {} / v {})",
                        st.sent_pkts, st.shown, st.agent_dropped, st.viewer_dropped
                    ));
                    ui.separator();
                    ui.label(format!(
                        "e2e {:.1}ms p95 {:.1}",
                        SessionStats::avg(&st.e2e_ms),
                        SessionStats::pct(&st.e2e_ms, 0.95)
                    ));
                    ui.separator();
                    ui.label(format!("rtt {:.2}ms", SessionStats::avg(&st.rtt_ms)));
                    ui.separator();
                    ui.label(format!("ctrl msgs: {}", st.control_msgs));
                    ui.separator();
                    let mons = monitors.lock().unwrap().clone();
                    let cur = info.lock().unwrap().as_ref().map(|s| s.active).unwrap_or(0);
                    if !mons.is_empty() {
                        ui.label("Монитор:");
                        for (idx, mw, mh) in &mons {
                            let sel = cur == *idx;
                            if ui
                                .selectable_label(sel, format!("{} ({}×{})", idx + 1, mw, mh))
                                .clicked()
                                && !sel
                            {
                                *pending_switch.lock().unwrap() = Some(*idx);
                            }
                        }
                    } else if let Some(si) = info.lock().unwrap().as_ref() {
                        ui.label(format!(
                            "монитор {}×{} @ ({},{})",
                            si.out.width, si.out.height, si.out.origin_x, si.out.origin_y
                        ));
                    }
                });
            });

            egui::Panel::bottom("hint").show(ui, |ui| {
                let st = self.stats.lock().unwrap();
                if !st.last_error.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(240, 120, 120),
                        format!("last error: {}", st.last_error),
                    );
                } else if let Some(secs) = self.smoke {
                    ui.label(format!(
                        "smoke: осталось {:.0} c",
                        (secs - self.started.elapsed().as_secs_f32()).max(0.0)
                    ));
                } else {
                    ui.label(
                        "P3-пилот: видео + ввод (мышь/клава/курсор). \
                         Мышь — над видео; клавиатура — когда окно в фокусе. \
                         Закройте окно для остановки.",
                    );
                }
            });

            let mut video_rect: Option<egui::Rect> = None;
            egui::CentralPanel::default().show(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                ui.painter()
                    .rect_filled(rect, 2.0, egui::Color32::from_rgb(12, 14, 18));
                if let (Some(tex), Some(f)) = (&self.texture, &self.last) {
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

            // Захват ввода над видео и отрисовка удалённого курсора (P3).
            if let Some(draw) = video_rect {
                let resp = ui.allocate_rect(draw, egui::Sense::click_and_drag());
                let mut locked = input_q.lock().unwrap();
                let focused = ui.ctx().input(|i| i.focused);

                // ---- Указатель: движения, кнопки, колесо ----
                let engaged =
                    resp.hovered() || resp.dragged() || self.buttons_down.iter().any(|d| *d);
                if engaged {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                    if let Some(p) = ui.ctx().input(|i| i.pointer.latest_pos()) {
                        let rel = p - draw.min;
                        let (nx, ny) =
                            input::ui_px_to_norm(rel.x, rel.y, draw.width(), draw.height());
                        if self.last_pointer != Some((nx, ny)) {
                            self.last_pointer = Some((nx, ny));
                            locked.push(InputEvent::Move { x: nx, y: ny });
                        }
                    }
                    let (mx, my) = self.last_pointer.unwrap_or((32768, 32768));
                    ui.ctx().input(|inp| {
                        let sd = inp.smooth_scroll_delta;
                        if resp.hovered() && (sd.x.abs() > 0.5 || sd.y.abs() > 0.5) {
                            locked.push(InputEvent::Wheel {
                                dx: sd.x.clamp(-32768.0, 32767.0) as i16,
                                dy: sd.y.clamp(-32768.0, 32767.0) as i16,
                                x: mx,
                                y: my,
                            });
                        }
                        for ev in &inp.events {
                            if let egui::Event::PointerButton {
                                pos,
                                button,
                                pressed,
                                ..
                            } = ev
                            {
                                let idx = match button {
                                    egui::PointerButton::Primary => 0usize,
                                    egui::PointerButton::Secondary => 1,
                                    egui::PointerButton::Middle => 2,
                                    egui::PointerButton::Extra1 => 3,
                                    egui::PointerButton::Extra2 => 4,
                                };
                                // Новое нажатие — только внутри видео;
                                // отпускание — всегда, если кнопку уже отправили зажатой.
                                if resp.hovered() || self.buttons_down[idx] {
                                    let rel = *pos - draw.min;
                                    let (nx, ny) = input::ui_px_to_norm(
                                        rel.x,
                                        rel.y,
                                        draw.width(),
                                        draw.height(),
                                    );
                                    self.last_pointer = Some((nx, ny));
                                    self.buttons_down[idx] = *pressed;
                                    locked.push(InputEvent::Button {
                                        btn: idx as u8,
                                        down: *pressed,
                                        x: nx,
                                        y: ny,
                                    });
                                }
                            }
                        }
                    });
                }

                // ---- Клавиатура: пока окно в фокусе ----
                if focused {
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
                                        // Печатные символы без Ctrl/Alt идут через Text-событие;
                                        // здесь — служебные клавиши и горячие комбинации.
                                        let shortcut = modifiers.ctrl || modifiers.alt;
                                        if shortcut || !is_char_vk(vk) {
                                            locked.push(InputEvent::Key { vk, down: *pressed });
                                        }
                                    }
                                }
                                egui::Event::Text(t) => {
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
                                // egui перехватывает Ctrl+C/X/V сам — транслируем на удалённую машину.
                                egui::Event::Copy => {
                                    locked.push(InputEvent::Key {
                                        vk: 0x43,
                                        down: true,
                                    });
                                    locked.push(InputEvent::Key {
                                        vk: 0x43,
                                        down: false,
                                    });
                                }
                                egui::Event::Cut => {
                                    locked.push(InputEvent::Key {
                                        vk: 0x58,
                                        down: true,
                                    });
                                    locked.push(InputEvent::Key {
                                        vk: 0x58,
                                        down: false,
                                    });
                                }
                                egui::Event::Paste(_) => {
                                    locked.push(InputEvent::Key {
                                        vk: 0x56,
                                        down: true,
                                    });
                                    locked.push(InputEvent::Key {
                                        vk: 0x56,
                                        down: false,
                                    });
                                }
                                _ => {}
                            }
                        }
                    });
                    // Модификаторы egui отдаёт состоянием, а не событием — шлём рёбра.
                    let m = ui.ctx().input(|i| i.modifiers);
                    let pm = self.prev_mods;
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
                    self.prev_mods = m;
                }

                // ---- Потеря фокуса: отпустить зажатое, чтобы не «залипло» на удалённой машине ----
                if !focused {
                    if self.buttons_down.iter().any(|d| *d) {
                        for (idx, d) in self.buttons_down.iter_mut().enumerate() {
                            if *d {
                                *d = false;
                                let (x, y) = self.last_pointer.unwrap_or((32768, 32768));
                                locked.push(InputEvent::Button {
                                    btn: idx as u8,
                                    down: false,
                                    x,
                                    y,
                                });
                            }
                        }
                    }
                    let pm = self.prev_mods;
                    for (was_on, vk) in [(pm.shift, 0x10u16), (pm.ctrl, 0x11), (pm.alt, 0x12)] {
                        if was_on {
                            locked.push(InputEvent::Key { vk, down: false });
                        }
                    }
                    self.prev_mods = egui::Modifiers::default();
                }
                drop(locked);

                // Удалённый курсор поверх видео.
                let cs = *cursor.lock().unwrap();
                if cs.visible {
                    let (px, py) = input::norm_to_ui_px(cs.x, cs.y, draw.width(), draw.height());
                    let pos = egui::pos2(draw.min.x + px, draw.min.y + py);
                    ui.painter()
                        .circle_filled(pos, 5.0, egui::Color32::from_black_alpha(180));
                    ui.painter().circle_filled(pos, 2.5, egui::Color32::WHITE);
                }
            }

            if let Some(secs) = self.smoke
                && self.started.elapsed().as_secs_f32() >= secs
                && !self.finished
            {
                self.finish();
            }

            if self.finished {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(10));
            }
        }
    }

    impl Drop for App {
        fn drop(&mut self) {
            self.finish();
        }
    }

    /// VK печатных клавиш: их доставляет Text-событие, по Key-пути не дублируем.
    fn is_char_vk(vk: u16) -> bool {
        matches!(vk, 0x20 | 0x30..=0x39 | 0x41..=0x5A)
    }

    /// egui-клавиша → Windows VK (для непечатаемых; печатные идут через vk_for_char).
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
}
