//! Служба Windows для relay: работает без входа пользователя и переживает
//! перезагрузки системы.
//!
//! ```text
//! rat-relay.exe --service            # работа под управлением SCM (запускает служба)
//! rat-relay.exe --install-service    # зарегистрировать службу «RatNextRelay» (автозапуск)
//! rat-relay.exe --uninstall-service  # остановить и удалить службу
//! ```
//!
//! Служба крутит тот же headless-режим, что и `--smoke`, но бессрочно, и ведёт
//! журнал в `relay.log` рядом с exe (у службы нет консоли).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::time::Duration;

/// Имя службы в SCM.
const SERVICE_NAME: &str = "RatNextRelay";
const DISPLAY_NAME: &str = "Rat Next relay";
const DESCRIPTION: &str =
    "Rat Next relay — связывает клиентов интернет-режима по ID (форвард сессий).";

static STOP: AtomicBool = AtomicBool::new(false);
static STATUS_HANDLE: AtomicIsize = AtomicIsize::new(0);
static CUR_STATE: AtomicU32 = AtomicU32::new(SERVICE_START_PENDING);

const SERVICE_WIN32_OWN_PROCESS: u32 = 0x0000_0010;
const SERVICE_AUTO_START: u32 = 0x0000_0002;
const SERVICE_ERROR_NORMAL: u32 = 0x0000_0001;
const SERVICE_CONTROL_STOP: u32 = 0x0000_0001;
const SERVICE_CONTROL_INTERROGATE: u32 = 0x0000_0004;
const SERVICE_ACCEPT_STOP: u32 = 0x0000_0001;
const SERVICE_STOPPED: u32 = 0x0000_0001;
const SERVICE_START_PENDING: u32 = 0x0000_0002;
const SERVICE_STOP_PENDING: u32 = 0x0000_0003;
const SERVICE_RUNNING: u32 = 0x0000_0004;

const SC_MANAGER_ALL_ACCESS: u32 = 0x000F_003F;
const SERVICE_ALL_ACCESS: u32 = 0x000F_01FF;
const SERVICE_CONFIG_DESCRIPTION: u32 = 1;
const SERVICE_CONFIG_FAILURE_ACTIONS: u32 = 2;
const SC_ACTION_RESTART: u32 = 1;
const ERROR_SERVICE_EXISTS: u32 = 1073;
const ERROR_SERVICE_ALREADY_RUNNING: u32 = 1056;
const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;

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

#[repr(C)]
struct ServiceTableEntry {
    name: *mut u16,
    proc: Option<unsafe extern "system" fn(u32, *mut *mut u16)>,
}

#[repr(C)]
struct ServiceDescriptionW {
    description: *mut u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ScAction {
    action_type: u32,
    delay: u32,
}

#[repr(C)]
struct ServiceFailureActionsW {
    reset_period: u32,
    reboot_msg: *mut u16,
    command: *mut u16,
    actions_count: u32,
    actions: *const ScAction,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Строка в `relay.log` рядом с exe (журнал службы).
fn log_line(message: &str) {
    use std::io::Write;
    let Some(path) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("relay.log")))
    else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "{ts} {message}");
    }
}

fn report(state: u32, checkpoint: u32, wait_hint: u32) {
    let handle = STATUS_HANDLE.load(Ordering::Relaxed);
    if handle == 0 {
        return;
    }
    let accepted = if state == SERVICE_RUNNING {
        SERVICE_ACCEPT_STOP
    } else {
        0
    };
    let status = ServiceStatus {
        service_type: SERVICE_WIN32_OWN_PROCESS,
        current_state: state,
        controls_accepted: accepted,
        win32_exit_code: 0,
        service_specific_exit_code: 0,
        checkpoint,
        wait_hint,
    };
    unsafe { SetServiceStatus(handle as *mut c_void, &status) };
    CUR_STATE.store(state, Ordering::Relaxed);
}

unsafe extern "system" fn handler(control: u32) {
    match control {
        SERVICE_CONTROL_STOP => {
            report(SERVICE_STOP_PENDING, 1, 5_000);
            STOP.store(true, Ordering::Relaxed);
        }
        SERVICE_CONTROL_INTERROGATE => {
            report(CUR_STATE.load(Ordering::Relaxed), 0, 0);
        }
        _ => {}
    }
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut *mut u16) {
    let name = wide(SERVICE_NAME);
    let handle = unsafe { RegisterServiceCtrlHandlerW(name.as_ptr(), Some(handler)) };
    STATUS_HANDLE.store(handle as isize, Ordering::Relaxed);
    report(SERVICE_START_PENDING, 1, 5_000);

    let configured = crate::settings::load();
    let addr = configured.listen_addr;
    match crate::core::Handle::start(&addr) {
        Ok(core_handle) => {
            report(SERVICE_RUNNING, 0, 0);
            log_line(&format!("служба запущена: {addr}"));
            run_loop(core_handle);
        }
        Err(e) => {
            log_line(&format!("не удалось запустить: {e}"));
        }
    }
    STOP.store(false, Ordering::Relaxed);
    report(SERVICE_STOPPED, 0, 0);
}

/// Основной цикл службы: журналируем события и ждём команду «стоп».
fn run_loop(handle: crate::core::Handle) {
    let stop = handle.stop.clone();
    while !STOP.load(Ordering::Relaxed) && !handle.is_finished() {
        match handle.events.recv_timeout(Duration::from_millis(200)) {
            Ok(crate::core::Event::Log(line)) => log_line(&line),
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
    }
    stop.store(true, Ordering::Relaxed);
    handle.join();
    log_line("служба остановлена");
}

/// Точка входа службы: отдаём управление SCM.
pub fn run_service() -> Result<(), String> {
    let name = wide(SERVICE_NAME);
    let table = [
        ServiceTableEntry {
            name: name.as_ptr() as *mut u16,
            proc: Some(service_main),
        },
        ServiceTableEntry {
            name: std::ptr::null_mut(),
            proc: None,
        },
    ];
    let ok = unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        return Err(format!("StartServiceCtrlDispatcher: ошибка {err}"));
    }
    Ok(())
}

/// Зарегистрировать службу (автозапуск при загрузке), запустить её.
pub fn install() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bin_path = format!("\"{}\" --service", exe.display());
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_ALL_ACCESS);
        if scm.is_null() {
            return Err(format!(
                "OpenSCManager: ошибка {} (нужны права администратора)",
                GetLastError()
            ));
        }
        let name = wide(SERVICE_NAME);
        let display = wide(DISPLAY_NAME);
        let bin = wide(&bin_path);
        let mut service = CreateServiceW(
            scm,
            name.as_ptr(),
            display.as_ptr(),
            SERVICE_ALL_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_AUTO_START,
            SERVICE_ERROR_NORMAL,
            bin.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
        );
        if service.is_null() {
            let err = GetLastError();
            if err != ERROR_SERVICE_EXISTS {
                CloseServiceHandle(scm);
                return Err(format!("CreateService: ошибка {err}"));
            }
            service = OpenServiceW(scm, name.as_ptr(), SERVICE_ALL_ACCESS);
            if service.is_null() {
                let err2 = GetLastError();
                CloseServiceHandle(scm);
                return Err(format!("OpenService: ошибка {err2}"));
            }
        }

        let desc = wide(DESCRIPTION);
        let sd = ServiceDescriptionW {
            description: desc.as_ptr() as *mut u16,
        };
        ChangeServiceConfig2W(
            service,
            SERVICE_CONFIG_DESCRIPTION,
            &sd as *const _ as *const c_void,
        );

        let actions = [ScAction {
            action_type: SC_ACTION_RESTART,
            delay: 5_000,
        }; 3];
        let fa = ServiceFailureActionsW {
            reset_period: 86_400,
            reboot_msg: std::ptr::null_mut(),
            command: std::ptr::null_mut(),
            actions_count: actions.len() as u32,
            actions: actions.as_ptr(),
        };
        ChangeServiceConfig2W(
            service,
            SERVICE_CONFIG_FAILURE_ACTIONS,
            &fa as *const _ as *const c_void,
        );

        if StartServiceW(service, 0, std::ptr::null()) == 0 {
            let err = GetLastError();
            if err != ERROR_SERVICE_ALREADY_RUNNING {
                log_line(&format!("StartService: ошибка {err}"));
            }
        }

        CloseServiceHandle(service);
        CloseServiceHandle(scm);
    }
    remove_autostart_task();
    log_line("служба установлена");
    Ok(())
}

/// Убрать задачу автозапуска «Rat Next relay» (при переходе на службу).
fn remove_autostart_task() {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("schtasks");
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let _ = cmd.args(["/Delete", "/F", "/TN", "Rat Next relay"]).output();
    }
}

/// Остановить и удалить службу.
pub fn uninstall() -> Result<(), String> {
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_ALL_ACCESS);
        if scm.is_null() {
            return Err(format!(
                "OpenSCManager: ошибка {} (нужны права администратора)",
                GetLastError()
            ));
        }
        let name = wide(SERVICE_NAME);
        let service = OpenServiceW(scm, name.as_ptr(), SERVICE_ALL_ACCESS);
        if service.is_null() {
            let err = GetLastError();
            CloseServiceHandle(scm);
            if err == ERROR_SERVICE_DOES_NOT_EXIST {
                return Ok(());
            }
            return Err(format!("OpenService: ошибка {err}"));
        }

        let mut status = ServiceStatus {
            service_type: 0,
            current_state: 0,
            controls_accepted: 0,
            win32_exit_code: 0,
            service_specific_exit_code: 0,
            checkpoint: 0,
            wait_hint: 0,
        };
        if ControlService(service, SERVICE_CONTROL_STOP, &mut status) != 0 {
            // Ждём остановки до ~10 секунд.
            for _ in 0..40 {
                if QueryServiceStatus(service, &mut status) == 0
                    || status.current_state == SERVICE_STOPPED
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }

        if DeleteService(service) == 0 {
            let err = GetLastError();
            CloseServiceHandle(service);
            CloseServiceHandle(scm);
            return Err(format!("DeleteService: ошибка {err}"));
        }
        CloseServiceHandle(service);
        CloseServiceHandle(scm);
    }
    Ok(())
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenSCManagerW(machine: *const u16, database: *const u16, access: u32) -> *mut c_void;
    #[allow(clippy::too_many_arguments)]
    fn CreateServiceW(
        scm: *mut c_void,
        name: *const u16,
        display: *const u16,
        access: u32,
        service_type: u32,
        start_type: u32,
        error_control: u32,
        bin_path: *const u16,
        load_order_group: *const u16,
        tag_id: *mut u32,
        dependencies: *const u16,
        account: *const u16,
        password: *const u16,
    ) -> *mut c_void;
    fn OpenServiceW(scm: *mut c_void, name: *const u16, access: u32) -> *mut c_void;
    fn StartServiceW(service: *mut c_void, argc: u32, argv: *const *const u16) -> i32;
    fn ControlService(service: *mut c_void, control: u32, status: *mut ServiceStatus) -> i32;
    fn QueryServiceStatus(service: *mut c_void, status: *mut ServiceStatus) -> i32;
    fn DeleteService(service: *mut c_void) -> i32;
    fn CloseServiceHandle(handle: *mut c_void) -> i32;
    fn ChangeServiceConfig2W(service: *mut c_void, info_level: u32, info: *const c_void) -> i32;
    fn StartServiceCtrlDispatcherW(table: *const ServiceTableEntry) -> i32;
    fn RegisterServiceCtrlHandlerW(
        name: *const u16,
        handler: Option<unsafe extern "system" fn(u32)>,
    ) -> *mut c_void;
    fn SetServiceStatus(handle: *mut c_void, status: &ServiceStatus) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLastError() -> u32;
}
