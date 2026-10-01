//! Спайк P0-Б: минимальный UI вьюера на egui/eframe (API egui 0.36:
//! `App::ui` + унифицированные `Panel::left/bottom` + `CentralPanel`).
//! Панель устройств (слева), область видео-заглушки (центр), строка статуса (снизу).
//!
//! Smoke-режим: `cargo run -- --smoke` — окно закрывается само через ~4 секунды.

use eframe::egui;

fn main() -> eframe::Result<()> {
    let smoke = std::env::args().any(|a| a == "--smoke");
    println!("UI_EGUI_STARTING smoke={smoke}");

    // На этой машине драйвер Intel падает в Vulkan-пути wgpu (igvk64.dll);
    // по умолчанию (если не задано извне) используем DX12-бэкенд.
    if std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: вызов до инициализации GUI, однопоточно.
        unsafe { std::env::set_var("WGPU_BACKEND", "dx12") };
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 700.0]),
        ..Default::default()
    };

    eframe::run_native(
        "rat-viewer-egui-spike",
        options,
        Box::new(move |_cc| Ok(Box::new(ViewerApp::new(smoke)))),
    )?;

    println!("UI_EGUI_SMOKE_OK");
    Ok(())
}

struct ViewerApp {
    devices: Vec<(String, String)>,
    selected: usize,
    smoke: bool,
    started: std::time::Instant,
}

impl ViewerApp {
    fn new(smoke: bool) -> Self {
        Self {
            devices: vec![
                ("000123".into(), "Касса №1 (магазин А)".into()),
                ("000456".into(), "Касса №2 (магазин А)".into()),
                ("000789".into(), "Бухгалтерия-ПК".into()),
                ("000321".into(), "Сервер-склад".into()),
            ],
            selected: 0,
            smoke,
            started: std::time::Instant::now(),
        }
    }
}

impl eframe::App for ViewerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::left("devices")
            .resizable(true)
            .default_size(250.0)
            .show(ui, |ui| {
                ui.heading("Устройства");
                ui.separator();
                for (i, (id, desc)) in self.devices.iter().enumerate() {
                    if ui
                        .selectable_label(self.selected == i, format!("{id}  {desc}"))
                        .clicked()
                    {
                        self.selected = i;
                    }
                }
            });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("● не подключено");
                ui.separator();
                ui.label("латентность: —");
                ui.separator();
                ui.label(if self.smoke {
                    "smoke-режим"
                } else {
                    "интерактив"
                });
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let (id, desc) = &self.devices[self.selected];
            ui.heading(format!("Экран: {id} — {desc}"));
            let rect = ui.available_rect_before_wrap();
            ui.painter()
                .rect_filled(rect, 4.0, egui::Color32::from_rgb(18, 22, 30));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "VIDEO PLACEHOLDER",
                egui::FontId::proportional(26.0),
                egui::Color32::from_gray(160),
            );
        });

        if self.smoke && self.started.elapsed().as_secs_f32() > 4.0 {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.ctx().request_repaint();
    }
}
