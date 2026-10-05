// Значок в области уведомлений (трей) для rat.exe.

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Команда, пришедшая из трея.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Show,
    Quit,
}

pub struct Tray {
    /// Владение значком: пока структура жива, значок виден в трее.
    _icon: TrayIcon,
    item_show: MenuItem,
    item_quit: MenuItem,
}

impl Tray {
    pub fn new() -> Result<Self, String> {
        let item_show = MenuItem::new("Показать окно", true, None);
        let item_quit = MenuItem::new("Выход", true, None);

        let menu = Menu::new();
        menu.append_items(&[&item_show, &PredefinedMenuItem::separator(), &item_quit])
            .map_err(|e| e.to_string())?;

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Rat Next")
            .with_icon(app_icon()?)
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            _icon: icon,
            item_show,
            item_quit,
        })
    }

    /// Прочитать накопившиеся команды трея (без блокировки).
    pub fn poll(&self) -> Option<Cmd> {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.item_show.id() {
                return Some(Cmd::Show);
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

/// Значок трея — тот же рисунок, что у окна и у `exe`.
fn app_icon() -> Result<Icon, String> {
    const SIDE: u32 = 32;
    Icon::from_rgba(crate::icon::rgba(SIDE), SIDE, SIDE).map_err(|e| e.to_string())
}
