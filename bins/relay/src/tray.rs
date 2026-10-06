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
    /// `quit_enabled = false` — хелпер службы: «Выход» в меню нет, завершать
    /// его через трей нельзя (служба сразу поднимет его снова).
    pub fn new(listen: &str, running: bool, quit_enabled: bool) -> Result<Self, String> {
        let item_state = MenuItem::new(format!("Адрес: {listen}"), false, None);
        let item_show = MenuItem::new("Показать окно", true, None);
        let item_toggle = MenuItem::new(toggle_text(running), true, None);
        let item_settings = MenuItem::new("Настройки…", true, None);
        let item_quit = MenuItem::new("Выход", true, None);

        let sep1 = PredefinedMenuItem::separator();
        let sep2 = PredefinedMenuItem::separator();
        let menu = Menu::new();
        if quit_enabled {
            menu.append_items(&[
                &item_state,
                &sep1,
                &item_show,
                &item_toggle,
                &item_settings,
                &sep2,
                &item_quit,
            ])
            .map_err(|e| e.to_string())?;
        } else {
            menu.append_items(&[&item_state, &sep1, &item_show, &item_toggle, &item_settings])
                .map_err(|e| e.to_string())?;
        }

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

/// Иконка relay в трее: зелёный «хаб» — работает, серый — остановлен.
fn state_icon(running: bool) -> Result<Icon, String> {
    const SIZE: u32 = 32;
    Icon::from_rgba(crate::icon::rgba_state(SIZE, running), SIZE, SIZE).map_err(|e| e.to_string())
}
