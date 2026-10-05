//! Иконка в системном трее с контекстным меню.

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Команда, пришедшая из трея.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Show,
    Toggle,
    Settings,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    item_state: MenuItem,
    item_show: MenuItem,
    item_toggle: MenuItem,
    item_settings: MenuItem,
    item_quit: MenuItem,
}

impl Tray {
    pub fn new(listen: &str, running: bool) -> Result<Self, String> {
        let item_state = MenuItem::new(format!("Адрес: {listen}"), false, None);
        let item_show = MenuItem::new("Показать окно", true, None);
        let item_toggle = MenuItem::new(toggle_text(running), true, None);
        let item_settings = MenuItem::new("Настройки…", true, None);
        let item_quit = MenuItem::new("Выход", true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &item_state,
            &PredefinedMenuItem::separator(),
            &item_show,
            &item_toggle,
            &item_settings,
            &PredefinedMenuItem::separator(),
            &item_quit,
        ])
        .map_err(|e| e.to_string())?;

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(tooltip(listen, running))
            .with_icon(state_icon(running)?)
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            icon,
            item_state,
            item_show,
            item_toggle,
            item_settings,
            item_quit,
        })
    }

    /// Обновить иконку и подписи под текущее состояние relay.
    pub fn set_state(&self, listen: &str, running: bool) {
        self.item_state.set_text(format!("Адрес: {listen}"));
        self.item_toggle.set_text(toggle_text(running));
        if let Ok(icon) = state_icon(running) {
            let _ = self.icon.set_icon(Some(icon));
        }
        let _ = self.icon.set_tooltip(Some(tooltip(listen, running)));
    }

    /// Прочитать накопившиеся команды трея (без блокировки).
    pub fn poll(&self) -> Option<Cmd> {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.item_show.id() {
                return Some(Cmd::Show);
            }
            if event.id == self.item_toggle.id() {
                return Some(Cmd::Toggle);
            }
            if event.id == self.item_settings.id() {
                return Some(Cmd::Settings);
            }
            if event.id == self.item_quit.id() {
                return Some(Cmd::Quit);
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            match event {
                TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                } => return Some(Cmd::Show),
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => return Some(Cmd::Show),
                _ => {}
            }
        }
        None
    }
}

fn toggle_text(running: bool) -> &'static str {
    if running {
        "Остановить relay"
    } else {
        "Запустить relay"
    }
}

fn tooltip(listen: &str, running: bool) -> String {
    let state = if running {
        "работает"
    } else {
        "остановлен"
    };
    format!("Rat Next relay — {state} ({listen})")
}

/// Кружок в трее: зелёный — работает, серый — остановлен.
fn state_icon(running: bool) -> Result<Icon, String> {
    const SIZE: usize = 32;
    let (r, g, b) = if running {
        (76u8, 175u8, 80u8)
    } else {
        (140u8, 146u8, 156u8)
    };
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    let center = (SIZE as f32 - 1.0) / 2.0;
    let radius = SIZE as f32 / 2.0 - 1.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            if (dx * dx + dy * dy).sqrt() > radius {
                continue;
            }
            let i = (y * SIZE + x) * 4;
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = 255;
        }
    }
    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32).map_err(|e| e.to_string())
}
