//! Спайк P2: loopback-сессия «агент ↔ вьюер» через rat-mux (in-memory duplex).
//!
//! Агент: захват экрана -> H.264 (QSV) -> mux-кадры (Video) -> «сеть».
//! Вьюер: приём mux-кадров -> декод -> показ (egui/wgpu).
//! Control-канал: Hello / Ping→Pong (RTT).
//!
//! Запуск:   cargo run --release                 (интерактивно)
//!           cargo run --release -- --smoke=20    (20 c и авто-сводка)

mod agent;
mod ffi;
mod link;
mod stats;
mod viewer;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use stats::{SessionStats, TimeBase};

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let smoke_secs: Option<f32> = args
        .iter()
        .find_map(|a| a.strip_prefix("--smoke=").and_then(|s| s.parse::<f32>().ok()))
        .or_else(|| {
            if args.iter().any(|a| a == "--smoke") {
                Some(15.0)
            } else {
                None
            }
        });

    println!("SESSION_LOOPBACK_START smoke={smoke_secs:?}");

    if std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: до инициализации GUI, однопоточно.
        unsafe { std::env::set_var("WGPU_BACKEND", "dx12") };
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 820.0]),
        ..Default::default()
    };

    eframe::run_native(
        "rat session loopback P2",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(smoke_secs)))),
    )?;

    println!("SESSION_LOOPBACK_DONE");
    Ok(())
}

struct App {
    rx: Receiver<viewer::FrameOut>,
    stats: Arc<Mutex<SessionStats>>,
    stop: Arc<AtomicBool>,
    handles: Vec<std::thread::JoinHandle<()>>,
    base: TimeBase,
    texture: Option<egui::TextureHandle>,
    last: Option<viewer::FrameOut>,
    started: Instant,
    smoke: Option<f32>,
    finished: bool,
    ui_e2e_ms: Vec<f32>,
}

impl App {
    fn new(smoke: Option<f32>) -> Self {
        let (agent_link, viewer_link) = link::duplex_pair();
        let stats = Arc::new(Mutex::new(SessionStats::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let base = TimeBase(Instant::now());
        let (tx, rx) = sync_channel::<viewer::FrameOut>(3);

        let mut handles: Vec<std::thread::JoinHandle<()>> = Vec::new();
        let codec = std::env::var("VIDEO_CODEC").unwrap_or_else(|_| "h264_qsv".into());
        let cfg = agent::AgentConfig {
            fps: 30,
            bitrate: 8_000_000,
            gop: 60,
            codec,
            preset: "veryfast".into(),
            async_depth: 1,
            low_delay: 1,
        };
        match agent::spawn_agent(base, stop.clone(), stats.clone(), agent_link, cfg) {
            Ok(mut hs) => handles.append(&mut hs),
            Err(e) => {
                let mut st = stats.lock().unwrap();
                st.agent_errors += 1;
                st.last_error = e;
            }
        }
        handles.push(viewer::spawn_viewer(
            base,
            stop.clone(),
            stats.clone(),
            viewer_link,
            tx,
        ));

        Self {
            rx,
            stats,
            stop,
            handles,
            base,
            texture: None,
            last: None,
            started: Instant::now(),
            smoke,
            finished: false,
            ui_e2e_ms: Vec::new(),
        }
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.stop.store(true, Ordering::Relaxed);
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
        {
            let st = self.stats.lock().unwrap();
            println!("=== SESSION SUMMARY ===");
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
                    let now = stats::now_us(&self.base);
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
                    self.texture =
                        Some(ui.ctx().load_texture("video", data, egui::TextureOptions::LINEAR));
                }
            }
        }

        egui::Panel::top("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let st = self.stats.lock().unwrap();
                ui.label(format!(
                    "codec: {}",
                    if st.codec.is_empty() { "…" } else { &st.codec }
                ));
                ui.separator();
                ui.label(format!("fps: {:.1}", st.fps));
                ui.separator();
                ui.label(format!(
                    "frame #{}",
                    self.last.as_ref().map(|f| f.id).unwrap_or(0)
                ));
                ui.separator();
                ui.label(format!(
                    "sent {} | shown {} | dropped {}",
                    st.sent_pkts, st.shown, st.dropped
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
                    "Loopback-сессия: агент → rat-mux → вьюер (в одном процессе). \
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
