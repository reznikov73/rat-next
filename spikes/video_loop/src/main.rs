//! Спайк P1: локальный видео-луп без сети.
//! Экран -> захват (DXGI) -> H.264 (h264_qsv, fallback libx264) -> декод -> показ (egui).
//! Замеряет стадии конвейера и показывает их в строке статуса.
//!
//! Запуск:  cargo run                 (обычный режим)
//!          cargo run -- --smoke      (12 секунд и авто-закрытие с печатью сводки)
//!          cargo run -- --smoke=20   (столько секунд)
//! Кодек можно переопределить:  $env:VIDEO_CODEC="libx264"

mod ffi;
mod pipeline;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use pipeline::{FrameOut, PipelineConfig, SharedStats, StageSamples};

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let smoke_secs: Option<f32> = args
        .iter()
        .find_map(|a| a.strip_prefix("--smoke=").and_then(|s| s.parse::<f32>().ok()))
        .or_else(|| {
            if args.iter().any(|a| a == "--smoke") {
                Some(12.0)
            } else {
                None
            }
        });

    println!("VIDEO_LOOP_START smoke={smoke_secs:?}");

    // На машинах с проблемным Vulkan-драйвером Intel — DX12.
    if std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: вызов до инициализации GUI, однопоточно.
        unsafe { std::env::set_var("WGPU_BACKEND", "dx12") };
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 820.0]),
        ..Default::default()
    };

    eframe::run_native(
        "rat video-loop P1",
        options,
        Box::new(move |_cc| Ok(Box::new(ViewerApp::new(smoke_secs)))),
    )?;

    println!("VIDEO_LOOP_DONE");
    Ok(())
}

struct ViewerApp {
    rx: Receiver<FrameOut>,
    stats: Arc<Mutex<SharedStats>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    texture: Option<egui::TextureHandle>,
    last_frame: Option<FrameOut>,
    started: Instant,
    smoke: Option<f32>,
    finished: bool,
    present_ms: Vec<f32>,
    e2e_wall_ms: Vec<f32>,
}

impl ViewerApp {
    fn new(smoke: Option<f32>) -> Self {
        let (tx, rx) = sync_channel::<FrameOut>(3);
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Mutex::new(SharedStats::default()));

        let codec = std::env::var("VIDEO_CODEC").unwrap_or_else(|_| "h264_qsv".into());
        let cfg = PipelineConfig {
            codec,
            fps: 30,
            bitrate: 8_000_000,
            gop: 60,
            preset: "veryfast".into(),
            async_depth: 1,
            low_delay: 1,
        };

        let stop2 = stop.clone();
        let stats2 = stats.clone();
        let worker = std::thread::spawn(move || {
            if let Err(e) = pipeline::run_pipeline(stop2.clone(), stats2.clone(), tx, cfg) {
                let mut st = stats2.lock().unwrap();
                st.errors += 1;
                st.last_error = e;
            }
        });

        Self {
            rx,
            stats,
            stop,
            worker: Some(worker),
            texture: None,
            last_frame: None,
            started: Instant::now(),
            smoke,
            finished: false,
            present_ms: Vec::new(),
            e2e_wall_ms: Vec::new(),
        }
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        let st = self.stats.lock().unwrap();
        println!("=== PIPELINE SUMMARY ===");
        println!("{}", st.summary());
        if !st.last_error.is_empty() {
            println!("last_error: {}", st.last_error);
        }
        drop(st);
        if !self.present_ms.is_empty() {
            println!(
                "present gap: avg {:.2}ms p95 {:.2}ms | e2e wall (capture->present): avg {:.2}ms p95 {:.2}ms",
                StageSamples::avg(&self.present_ms),
                StageSamples::pct(&self.present_ms, 0.95),
                StageSamples::avg(&self.e2e_wall_ms),
                StageSamples::pct(&self.e2e_wall_ms, 0.95),
            );
        }
    }
}

impl eframe::App for ViewerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // принять самый свежий кадр (остальные сбрасываем — нам важна свежесть)
        loop {
            match self.rx.try_recv() {
                Ok(f) => {
                    let now = Instant::now();
                    push_capped(&mut self.present_ms, (now - f.t_decoded).as_secs_f32() * 1000.0);
                    push_capped(&mut self.e2e_wall_ms, (now - f.t_capture).as_secs_f32() * 1000.0);
                    self.last_frame = Some(f);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        // обновить текстуру
        if let Some(f) = &self.last_frame {
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

        // верхняя строка статуса
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
                    self.last_frame.as_ref().map(|f| f.id).unwrap_or(0)
                ));
                ui.separator();
                ui.label(format!("frames: {}", st.frames));
                ui.separator();
                ui.label(format!("dropped: {}", st.dropped));
                ui.separator();
                ui.label(format!("errors: {}", st.errors));
                ui.separator();
                let s = &st.samples;
                ui.label(format!(
                    "cap {:.1} | enc {:.1} | dec {:.1} | e2e {:.1}ms (p95 {:.1}ms)",
                    StageSamples::avg(&s.cap),
                    StageSamples::avg(&s.enc),
                    StageSamples::avg(&s.dec),
                    StageSamples::avg(&s.e2e),
                    StageSamples::pct(&s.e2e, 0.95),
                ));
            });
        });

        // нижняя строка
        egui::Panel::bottom("hint").show(ui, |ui| {
            let st = self.stats.lock().unwrap();
            if !st.last_error.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(240, 120, 120),
                    format!("last error: {}", st.last_error),
                );
            } else if let Some(secs) = self.smoke {
                ui.label(format!(
                    "smoke-режим: осталось {:.0} c, затем авто-закрытие и печать сводки",
                    (secs - self.started.elapsed().as_secs_f32()).max(0.0)
                ));
            } else {
                ui.label("Локальный луп: экран → QSV H.264 → декод → показ. Закройте окно для остановки.");
            }
        });

        // видео-область
        egui::CentralPanel::default().show(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            ui.painter()
                .rect_filled(rect, 2.0, egui::Color32::from_rgb(12, 14, 18));
            if let (Some(tex), Some(f)) = (&self.texture, &self.last_frame) {
                let img_ar = f.w as f32 / f.h as f32;
                let rect_ar = rect.width() / rect.height().max(1.0);
                let draw_rect = if img_ar > rect_ar {
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
                    draw_rect,
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

        // smoke: завершение
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

impl Drop for ViewerApp {
    fn drop(&mut self) {
        self.finish();
    }
}

fn push_capped(v: &mut Vec<f32>, x: f32) {
    v.push(x);
    if v.len() > 600 {
        v.remove(0);
    }
}
