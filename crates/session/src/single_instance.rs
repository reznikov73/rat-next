//! Защита от двойного запуска процесса: именованный мьютекс Windows.
//!
//! Имя привязано к пути exe — копии приложения в разных папках не мешают
//! друг другу, а повторный запуск того же файла блокируется.

/// Попытаться стать единственным экземпляром приложения.
/// Возвращает `false`, если экземпляр уже запущен.
pub fn try_acquire(tag: &str) -> bool {
    #[cfg(windows)]
    {
        let name = mutex_name(tag);
        let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
        if handle.is_null() {
            // Не смогли создать мьютекс — не мешаем запуску.
            return true;
        }
        // ВАЖНО: хэндл не закрываем — мьютекс живёт до конца процесса
        // (сырой указатель без деструктора: просто не вызываем CloseHandle).
        let _ = handle;
        let last_error = unsafe { GetLastError() };
        last_error != ERROR_ALREADY_EXISTS
    }
    #[cfg(not(windows))]
    {
        let _ = tag;
        true
    }
}

/// Показать информационное окно (при повторном запуске). Блокирует до «ОК».
pub fn warn(title: &str, text: &str) {
    #[cfg(windows)]
    unsafe {
        let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let text: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            0x0000_0040, // MB_OK | MB_ICONINFORMATION
        );
    }
    #[cfg(not(windows))]
    {
        let _ = (title, text);
    }
}

/// Слушатель запроса «показать окно» от повторного запуска.
pub struct ShowListener {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ShowListener {
    /// Забрать накопившийся запрос (при чтении сбрасывается).
    pub fn take(&self) -> bool {
        self.flag.swap(false, std::sync::atomic::Ordering::SeqCst)
    }
}

/// Попросить работающий экземпляр показать своё окно (то же, что клик по
/// значку в трее; работает и когда значка нет). `true` — запрос доставлен.
pub fn show_signal(tag: &str) -> bool {
    #[cfg(windows)]
    {
        let name = event_name(tag);
        let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        if handle.is_null() {
            return false;
        }
        let ok = unsafe { SetEvent(handle) } != 0;
        unsafe { CloseHandle(handle) };
        ok
    }
    #[cfg(not(windows))]
    {
        let _ = tag;
        false
    }
}

/// Создать событие «показать окно» и слушающий поток. `wake` вызывается после
/// каждого запроса — им будим перерисовку интерфейса.
pub fn show_listener(tag: &str, wake: impl Fn() + Send + 'static) -> Option<ShowListener> {
    #[cfg(windows)]
    {
        let name = event_name(tag);
        // Авто-сброс (manual_reset = 0): один запрос — одно взведение.
        let handle = unsafe { CreateEventW(std::ptr::null_mut(), 0, 0, name.as_ptr()) };
        if handle.is_null() {
            return None;
        }
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag2 = flag.clone();
        // Сырой описатель не Send — переносим числом, в потоке восстанавливаем.
        let handle_num = handle as usize;
        std::thread::spawn(move || {
            let handle = handle_num as *mut std::ffi::c_void;
            // Хэндл живёт до конца процесса — поток можно не завершать.
            loop {
                if unsafe { WaitForSingleObject(handle, INFINITE) } != 0 {
                    break;
                }
                flag2.store(true, std::sync::atomic::Ordering::SeqCst);
                wake();
            }
        });
        Some(ShowListener { flag })
    }
    #[cfg(not(windows))]
    {
        let _ = (tag, wake);
        None
    }
}

#[cfg(windows)]
fn exe_key() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .replace('\\', "_")
}

#[cfg(windows)]
fn mutex_name(tag: &str) -> Vec<u16> {
    format!("rat-next-single-{tag}-{}", exe_key())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn event_name(tag: &str) -> Vec<u16> {
    format!("rat-next-show-{tag}-{}", exe_key())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
const ERROR_ALREADY_EXISTS: u32 = 183;

#[cfg(windows)]
const EVENT_MODIFY_STATE: u32 = 0x0002;

#[cfg(windows)]
const INFINITE: u32 = 0xFFFF_FFFF;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(
        attrs: *mut std::ffi::c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn CreateEventW(
        attrs: *mut std::ffi::c_void,
        manual_reset: i32,
        initial_state: i32,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> *mut std::ffi::c_void;
    fn SetEvent(handle: *mut std::ffi::c_void) -> i32;
    fn WaitForSingleObject(handle: *mut std::ffi::c_void, ms: u32) -> u32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    fn GetLastError() -> u32;
}

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBoxW(
        hwnd: *mut std::ffi::c_void,
        text: *const u16,
        caption: *const u16,
        u_type: u32,
    ) -> i32;
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails() {
        let tag = format!("unit-{}", std::process::id());
        assert!(try_acquire(&tag));
        assert!(!try_acquire(&tag));
    }
}
