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

#[cfg(windows)]
fn mutex_name(tag: &str) -> Vec<u16> {
    let path = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .replace('\\', "_");
    format!("rat-next-single-{tag}-{path}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
const ERROR_ALREADY_EXISTS: u32 = 183;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(
        attrs: *mut std::ffi::c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
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
