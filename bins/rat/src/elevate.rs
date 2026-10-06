//! Запуск себя с повышенными правами (запрос UAC) — для операций со службой.
//!
//! Используется из окна настроек: если обычная попытка установить/остановить/
//! удалить службу отклонена (нужны права администратора), интерфейс запускает
//! `rat.exe` с тем же флагом через `ShellExecuteW("runas", ...)`. Windows
//! показывает запрос прав; после подтверждения операция выполняется в новом
//! процессе и он сразу выходит.

use std::ffi::c_void;

/// Запустить `rat.exe <flag>` с запросом прав администратора.
/// `Ok(())` — пользователь подтвердил (процесс запущен), иначе — отмена/ошибка.
pub fn relaunch(flag: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_w = wide(&exe.to_string_lossy());
    let params_w = wide(flag);
    let verb_w = wide("runas");
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb_w.as_ptr(),
            exe_w.as_ptr(),
            params_w.as_ptr(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    } as isize;
    if result <= 32 {
        Err(format!(
            "запуск от имени администратора отклонён (код {result})"
        ))
    } else {
        Ok(())
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        hwnd: *mut c_void,
        verb: *const u16,
        file: *const u16,
        params: *const u16,
        dir: *const u16,
        show: i32,
    ) -> *mut c_void;
}
