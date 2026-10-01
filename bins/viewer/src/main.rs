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
    use rat_session::link::{TcpCloser, tcp_connect};
    use rat_session::stats::{SessionStats, TimeBase};
    use rat_session::viewer::{FrameOut, spawn_viewer};

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
            let handle = spawn_viewer(base, stop.clone(), stats.clone(), link, tx);

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
                        "P2-пилот: сессия агент ↔ вьюер поверх TCP. \
                         Закройте окно для остановки.",
                    );
                }
            });

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

            if let Some(secs) = self.smoke {
                if self.started.elapsed().as_secs_f32() >= secs && !self.finished {
                    self.finish();
                }
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
}
