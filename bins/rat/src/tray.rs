// Значок в области уведомлений (трей) для rat.exe.

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Вариант исполнения — влияет на значок, подсказку и меню.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Помощник службы (SYSTEM, скрытый агент).
    Service,
    /// Обычный запуск из ярлыка.
    Manual,
}

/// Команда, пришедшая из трея.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Show,
    Settings,
    Quit,
}

pub struct Tray {
    /// Владение значком: пока структура жива, значок виден в трее.
    _icon: TrayIcon,
    item_show: MenuItem,
    item_settings: MenuItem,
    item_quit: Option<MenuItem>,
}

impl Tray {
    /// Создать значок для варианта `kind`.
    pub fn new(kind: Kind) -> Result<Self, String> {
        let item_show = MenuItem::new("Показать окно", true, None);
        let item_settings = MenuItem::new("Настройки…", true, None);
        let sep1 = PredefinedMenuItem::separator();

        let menu = Menu::new();
        menu.append_items(&[&item_show, &sep1, &item_settings])
            .map_err(|e| e.to_string())?;

        let mut item_quit = None;
        match kind {
            Kind::Service => {
                // Скрытый агент службы: «Выход» отключён — служба поднимет
                // агента снова, завершать его через трей нельзя.
            }
            Kind::Manual => {
                let sep2 = PredefinedMenuItem::separator();
                let quit = MenuItem::new("Выход", true, None);
                menu.append_items(&[&sep2, &quit])
                    .map_err(|e| e.to_string())?;
                item_quit = Some(quit);
            }
        }

        let (tooltip, variant) = match kind {
            Kind::Service => ("Rat Next — служба", crate::icon::IconVariant::Service),
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
        })
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

/// Уничтожить осиротевшие служебные окна значка (класс `tray_icon_app`).
///
/// Неудачная попытка регистрации значка оставляет такое окно «пустым» (без
/// значка), но при перезапуске проводника оно получает рассылку TaskbarCreated
/// и регистрирует значок заново — в панели появляется дубликат. Вызывать
/// можно только когда собственного значка нет (tray == None), иначе есть риск
/// уничтожить живой значок.
pub fn cleanup_orphans() {
    unsafe {
        EnumWindows(Some(close_orphan_cb), 0);
    }
}

/// Колбэк перечисления окон: закрыть «пустые» окна значка нашего процесса.
unsafe extern "system" fn close_orphan_cb(hwnd: *mut std::ffi::c_void, _lp: isize) -> i32 {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 0 {
        return 1;
    }
    if String::from_utf16_lossy(&buf[..n as usize]) != "tray_icon_app" {
        return 1;
    }
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == unsafe { GetCurrentProcessId() } {
        // WM_CLOSE через SendMessage: окно закрывается синхронно, до того как
        // очередь сообщений успеет доставить рассылку TaskbarCreated — иначе
        // «черновик» мог бы успеть зарегистрировать значок. DefWindowProc по
        // WM_CLOSE разрушает окно и освобождает данные.
        unsafe { SendMessageW(hwnd, 0x0010, 0, 0) };
    }
    1
}

#[link(name = "user32")]
unsafe extern "system" {
    fn EnumWindows(
        cb: Option<unsafe extern "system" fn(*mut std::ffi::c_void, isize) -> i32>,
        lparam: isize,
    ) -> i32;
    fn GetClassNameW(hwnd: *mut std::ffi::c_void, buf: *mut u16, max: i32) -> i32;
    fn GetWindowThreadProcessId(hwnd: *mut std::ffi::c_void, pid: *mut u32) -> u32;
    fn SendMessageW(hwnd: *mut std::ffi::c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcessId() -> u32;
}
