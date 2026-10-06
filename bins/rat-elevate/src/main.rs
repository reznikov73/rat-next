//! `rat-elevate` — мини-лаунчер для ярлыков «Rat Next».
//!
//! Если служба «RatNext» работает — целевое приложение запускается как обычно
//! (окно откроет уже работающий экземпляр через механизм повторного запуска).
//! Если службы нет, она остановлена или статус не выяснился — запускаем цель
//! с повышением прав (UAC); отмена запроса UAC — тихий выход без окон.
//!
//! Манифеста `requireAdministrator` нет специально: иначе UAC спрашивался бы
//! при каждом запуске, даже когда служба уже работает.

#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::ffi::c_void;
use std::path::{Path, PathBuf};

#[repr(C)]
struct ServiceStatus {
    service_type: u32,
    current_state: u32,
    controls_accepted: u32,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
    checkpoint: u32,
    wait_hint: u32,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let target = args.next().unwrap_or_else(|| "rat.exe".to_string());
    let rest: Vec<String> = args.collect();

    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    // Целевой exe ищем рядом с собой: аргумент — имя файла или путь.
    let target_path = if target.contains(['\\', '/']) {
        PathBuf::from(&target)
    } else {
        dir.join(&target)
    };

    if service_running("RatNext") {
        // Служба работает: обычный запуск, без UAC.
        let _ = std::process::Command::new(&target_path)
            .args(&rest)
            .current_dir(&dir)
            .spawn();
        return;
    }

    // Службы нет или она остановлена (или статус не выяснился): повышаем права.
    // Отмена UAC или ошибка — просто тихий выход.
    let _ = run_elevated(&target_path, &rest);
}

/// Работает ли служба с указанным именем. Любая ошибка — «нет».
fn service_running(name: &str) -> bool {
    const SC_MANAGER_CONNECT: u32 = 0x0001;
    const SERVICE_QUERY_STATUS: u32 = 0x0004;
    const SERVICE_RUNNING: u32 = 0x0004;
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return false;
        }
        let name_w = to_wide(name);
        let service = OpenServiceW(scm, name_w.as_ptr(), SERVICE_QUERY_STATUS);
        if service.is_null() {
            CloseServiceHandle(scm);
            return false;
        }
        let mut status: ServiceStatus = std::mem::zeroed();
        let ok = QueryServiceStatus(service, &mut status);
        CloseServiceHandle(service);
        CloseServiceHandle(scm);
        ok != 0 && status.current_state == SERVICE_RUNNING
    }
}

/// Запустить цель через ShellExecuteW("runas", …). Возвращает `Err` при отмене.
fn run_elevated(target: &Path, args: &[String]) -> Result<(), String> {
    let file = to_wide(&target.to_string_lossy());
    let params = to_wide(&quote_args(args));
    let verb = to_wide("runas");
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    } as isize;
    if result <= 32 {
        Err(format!("повышение прав отклонено (код {result})"))
    } else {
        Ok(())
    }
}

/// Кавычки для аргументов с пробелами (их получает командная строка Windows).
fn quote_args(args: &[String]) -> String {
    let mut out = String::new();
    for a in args {
        if !out.is_empty() {
            out.push(' ');
        }
        if a.contains([' ', '\t']) {
            out.push('"');
            out.push_str(a);
            out.push('"');
        } else {
            out.push_str(a);
        }
    }
    out
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenSCManagerW(machine: *const u16, database: *const u16, access: u32) -> *mut c_void;
    fn OpenServiceW(scm: *mut c_void, name: *const u16, access: u32) -> *mut c_void;
    fn QueryServiceStatus(service: *mut c_void, status: *mut ServiceStatus) -> i32;
    fn CloseServiceHandle(handle: *mut c_void) -> i32;
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
