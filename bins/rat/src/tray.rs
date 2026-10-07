// Значок в области уведомлений (трей) для rat.exe.

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Вариант исполнения — влияет на значок, подсказку и контекстное меню.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Помощник службы (SYSTEM, скрытый агент).
    Service,
    /// Агент автозапуска (запущен задачей планировщика, `--tray`).
    Agent,
    /// Обычный запуск из ярлыка.
    Manual,
}

/// Команда, пришедшая из трея.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Show,
    Settings,
    Quit,
    /// Переключить автозапуск (включить или выключить по состоянию).
    ToggleAutostart,
}

pub struct Tray {
    /// Владение значком: пока структура жива, значок виден в трее.
    _icon: TrayIcon,
    item_show: MenuItem,
    item_settings: MenuItem,
    item_quit: Option<MenuItem>,
    item_autostart: Option<MenuItem>,
}

impl Tray {
    /// Создать значок для варианта `kind`.
    ///
    /// `autostart_installed` — есть ли задача планировщика (подпись пункта);
    /// `service_installed` — установлена ли служба: тогда пункт автозапуска
    /// не показываем, автозапуском управляет служба.
    pub fn new(
        kind: Kind,
        autostart_installed: bool,
        service_installed: bool,
    ) -> Result<Self, String> {
        let item_show = MenuItem::new("Показать окно", true, None);
        let item_settings = MenuItem::new("Настройки…", true, None);
        let sep1 = PredefinedMenuItem::separator();

        let menu = Menu::new();
        menu.append_items(&[&item_show, &sep1, &item_settings])
            .map_err(|e| e.to_string())?;

        let mut item_quit = None;
        let mut item_autostart = None;
        match kind {
            Kind::Service => {
                // Скрытый агент службы: «Выход» отключён — служба поднимет
                // агента снова, завершать его через трей нельзя.
            }
            Kind::Agent | Kind::Manual => {
                if !service_installed {
                    let label = if autostart_installed {
                        "Отключить автозапуск"
                    } else {
                        "Включить автозапуск"
                    };
                    let item = MenuItem::new(label, true, None);
                    menu.append_items(&[&item]).map_err(|e| e.to_string())?;
                    item_autostart = Some(item);
                }
                let sep2 = PredefinedMenuItem::separator();
                let quit = MenuItem::new("Выход", true, None);
                menu.append_items(&[&sep2, &quit])
                    .map_err(|e| e.to_string())?;
                item_quit = Some(quit);
            }
        }

        let (tooltip, variant) = match kind {
            Kind::Service => ("Rat Next — служба", crate::icon::IconVariant::Service),
            Kind::Agent => ("Rat Next — автозапуск", crate::icon::IconVariant::Task),
            Kind::Manual => ("Rat Next", crate::icon::IconVariant::Manual),
        };

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(tooltip)
            .with_icon(app_icon(variant)?)
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            _icon: icon,
            item_show,
            item_settings,
            item_quit,
            item_autostart,
        })
    }

    /// Обновить подпись пункта автозапуска после переключения.
    pub fn set_autostart_text(&self, installed: bool) {
        if let Some(item) = &self.item_autostart {
            item.set_text(if installed {
                "Отключить автозапуск"
            } else {
                "Включить автозапуск"
            });
        }
    }

    /// Прочитать накопившиеся команды трея (без блокировки).
    pub fn poll(&self) -> Option<Cmd> {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.item_show.id() {
                return Some(Cmd::Show);
            }
            if event.id == self.item_settings.id() {
                return Some(Cmd::Settings);
            }
            if let Some(item) = &self.item_quit
                && event.id == item.id()
            {
                return Some(Cmd::Quit);
            }
            if let Some(item) = &self.item_autostart
                && event.id == item.id()
            {
                return Some(Cmd::ToggleAutostart);
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

/// Значок трея — базовый рисунок с акцентом варианта.
fn app_icon(variant: crate::icon::IconVariant) -> Result<Icon, String> {
    const SIDE: u32 = 32;
    Icon::from_rgba(crate::icon::rgba_variant(SIDE, variant), SIDE, SIDE).map_err(|e| e.to_string())
}
