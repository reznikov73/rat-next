//! Служба Windows для агента Rat Next.
//!
//! Служба стартует с загрузки системы (не дожидаясь входа пользователя) и
//! держит в активной пользовательской сессии «хелпер» — обычное приложение
//! Rat Next (`--service-helper --tray`), которое принимает подключения.
//! Хелпер запускается с системным токеном, поэтому ввод действует во всех
//! окнах, включая администраторские.
//!
//! ```text
//! rat.exe --service            # работа под управлением SCM (запускает служба)
//! rat.exe --install-service    # зарегистрировать службу «RatNext» и запустить
//! rat.exe --uninstall-service  # остановить и удалить службу
//! ```
//!
//! При установке службы задача автозапуска «Rat Next» удаляется, чтобы не
//! было двух конкурирующих агентов.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::time::Duration;

/// Имя службы в SCM.
const SERVICE_NAME: &str = "RatNext";
const DISPLAY_NAME: &str = "Rat Next";
const DESCRIPTION: &str =
    "Rat Next — агент удалённого доступа: доступен с момента загрузки Windows.";

static STOP: AtomicBool = AtomicBool::new(false);
static STATUS_HANDLE: AtomicIsize = AtomicIsize::new(0);
static CUR_STATE: AtomicU32 = AtomicU32::new(SERVICE_START_PENDING);
static HELPER_PID: AtomicU32 = AtomicU32::new(0);
static HELPER_SESSION: AtomicU32 = AtomicU32::new(u32::MAX);

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

const SC_MANAGER_CONNECT: u32 = 0x0001;
const SERVICE_QUERY_STATUS: u32 = 0x0004;
const SERVICE_START: u32 = 0x0010;
const SERVICE_STOP: u32 = 0x0020;
const ERROR_SERVICE_NOT_ACTIVE: u32 = 1062;

const TOKEN_ALL_ACCESS: u32 = 0x000F_01FF;
const WAIT_TIMEOUT: i32 = 0x0000_0102;

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

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    lp_reserved: *mut u16,
    lp_desktop: *mut u16,
    lp_title: *mut u16,
    dw_x: u32,
    dw_y: u32,
    dw_x_size: u32,
    dw_y_size: u32,
    dw_x_count_chars: u32,
    dw_y_count_chars: u32,
    dw_fill_attribute: u32,
    dw_flags: u32,
    w_show_window: u16,
    cb_reserved_2: u16,
    lp_reserved_2: *mut u8,
    h_std_input: *mut c_void,
    h_std_output: *mut c_void,
    h_std_error: *mut c_void,
}

#[repr(C)]
struct ProcessInformationW {
    h_process: *mut c_void,
    h_thread: *mut c_void,
    dw_process_id: u32,
    dw_thread_id: u32,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Строка в `rat-svc.log` рядом с exe (журнал службы).
fn log_line(message: &str) {
    use std::io::Write;
    let Some(path) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("rat-svc.log")))
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
    report(SERVICE_RUNNING, 0, 0);
    log_line("служба запущена");
    supervise();
    // Если из интерфейса попросили оставить помощника (остановка/удаление из
    // окна) — не убиваем его: окно продолжит работать как обычное приложение.
    let keep_path = keep_helper_path();
    let keep = keep_path.as_ref().is_some_and(|p| p.exists());
    if keep {
        if let Some(p) = &keep_path {
            let _ = std::fs::remove_file(p);
        }
        HELPER_PID.store(0, Ordering::Relaxed);
        HELPER_SESSION.store(u32::MAX, Ordering::Relaxed);
        log_line("хелпер оставлен работать (передано управление)");
    } else {
        kill_helper();
    }
    log_line("служба остановлена");
    STOP.store(false, Ordering::Relaxed);
    report(SERVICE_STOPPED, 0, 0);
}

/// Следим за активной сессией и держим в ней хелпер.
fn supervise() {
    let mut last_spawn: Option<std::time::Instant> = None;
    let mut fail_streak: u32 = 0;
    while !STOP.load(Ordering::Relaxed) {
        let session = unsafe { WTSGetActiveConsoleSessionId() };
        if session != u32::MAX {
            let pid = HELPER_PID.load(Ordering::Relaxed);
            let alive = pid != 0 && process_alive(pid);
            let session_changed = HELPER_SESSION.load(Ordering::Relaxed) != session;
            // Помощник прожил достаточно долго — считаем проблему решённой.
            if alive
                && !session_changed
                && last_spawn.is_some_and(|t| t.elapsed() >= Duration::from_secs(30))
            {
                fail_streak = 0;
            }
            if !alive || session_changed {
                if pid != 0 && !alive {
                    log_line(&format!("хелпер завершился: pid={pid}"));
                }
                kill_helper();
                // Если предыдущий хелпер умер сразу (например, уже работает
                // другой экземпляр приложения) — выдерживаем паузу, наращивая
                // её при повторных быстрых смертях: 8с → 15с → 30с → 60с.
                let fast_fail = last_spawn.is_some_and(|t| t.elapsed() < Duration::from_secs(3));
                if fast_fail {
                    fail_streak = fail_streak.saturating_add(1);
                    let delay = match fail_streak {
                        0 | 1 => 8,
                        2 => 15,
                        3 => 30,
                        _ => 60,
                    };
                    sleep_steps(Duration::from_secs(delay));
                    if STOP.load(Ordering::Relaxed) {
                        break;
                    }
                }
                match spawn_helper(session) {
                    Ok(pid) => {
                        HELPER_PID.store(pid, Ordering::Relaxed);
                        HELPER_SESSION.store(session, Ordering::Relaxed);
                        last_spawn = Some(std::time::Instant::now());
                        log_line(&format!("хелпер запущен: pid={pid}, сессия={session}"));
                    }
                    Err(e) => {
                        last_spawn = Some(std::time::Instant::now());
                        log_line(&format!("хелпер не запустился: {e}"));
                    }
                }
            }
        } else if HELPER_PID.load(Ordering::Relaxed) != 0 {
            log_line("активной сессии нет — хелпер остановлен");
            kill_helper();
        }
        sleep_steps(Duration::from_millis(1500));
    }
}

/// Сон с частыми проверками команды «стоп».
fn sleep_steps(total: Duration) {
    let step = Duration::from_millis(200);
    let mut left = total;
    while left > Duration::ZERO {
        if STOP.load(Ordering::Relaxed) {
            return;
        }
        let s = if left < step { left } else { step };
        std::thread::sleep(s);
        left -= s;
    }
}

fn process_alive(pid: u32) -> bool {
    unsafe {
        // SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION
        let handle = OpenProcess(0x0010_0000 | 0x0000_1000, 0, pid);
        if handle.is_null() {
            return false;
        }
        let result = WaitForSingleObject(handle, 0);
        CloseHandle(handle);
        result == WAIT_TIMEOUT
    }
}

/// Файл-просьба «не завершать помощника»: ставится интерфейсом при остановке
/// или удалении службы — окно должно остаться работать.
const KEEP_FILE: &str = "rat-keep-helper.tmp";

fn keep_helper_path() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(KEEP_FILE)))
}

/// Поставить просьбу «оставить помощника» (вызывает интерфейс приложения).
pub fn raise_keep_helper() {
    if let Some(p) = keep_helper_path() {
        let _ = std::fs::write(&p, b"1");
    }
}

/// Снять просьбу (после завершённой операции).
pub fn clear_keep_helper() {
    if let Some(p) = keep_helper_path() {
        let _ = std::fs::remove_file(&p);
    }
}

fn kill_helper() {
    let pid = HELPER_PID.swap(0, Ordering::Relaxed);
    let session = HELPER_SESSION.swap(u32::MAX, Ordering::Relaxed);
    if pid == 0 {
        return;
    }
    // Сначала по-хорошему: помощник закроется сам и штатно уберёт значок
    // трея (принудительная смерть оставляет «мёртвый» значок в панели).
    if process_alive(pid) && polite_quit(pid, session) {
        log_line(&format!("хелпер завершён вежливо: pid={pid}"));
        return;
    }
    unsafe {
        let handle = OpenProcess(0x0000_0001, 0, pid); // PROCESS_TERMINATE
        if !handle.is_null() {
            TerminateProcess(handle, 0);
            CloseHandle(handle);
        }
    }
    log_line(&format!("хелпер остановлен: pid={pid}"));
}

/// Вежливая остановка помощника: просим его завершиться самому (он снимет
/// значок трея) и ждём выхода до ~4 секунд. Сигнал действует только внутри
/// сессии, поэтому просьбу доставляет процесс, запущенный в сессии помощника.
/// `true` — помощник вышел сам.
fn polite_quit(pid: u32, session: u32) -> bool {
    if session == u32::MAX {
        return false;
    }
    if let Err(e) = spawn_in_session(session, "--request-quit") {
        log_line(&format!("вежливое завершение не доставлено: {e}"));
        return false;
    }
    unsafe {
        // SYNCHRONIZE
        let handle = OpenProcess(0x0010_0000, 0, pid);
        if handle.is_null() {
            return false;
        }
        let result = WaitForSingleObject(handle, 4_000);
        CloseHandle(handle);
        result == 0
    }
}

/// Запустить хелпер в сессии `session` с системным токеном.
fn spawn_helper(session: u32) -> Result<u32, String> {
    spawn_in_session(session, "--service-helper --tray")
}

/// Запустить наш exe с аргументами в сессии `session` (системный токен).
fn spawn_in_session(session: u32, args_line: &str) -> Result<u32, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let cmd = format!("\"{}\" {args_line}", exe.display());
    unsafe {
        let mut token: *mut c_void = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut token) == 0 {
            return Err(format!("OpenProcessToken: ошибка {}", GetLastError()));
        }
        let mut dup: *mut c_void = std::ptr::null_mut();
        let ok = DuplicateTokenEx(
            token,
            TOKEN_ALL_ACCESS,
            std::ptr::null_mut(),
            2, // SecurityImpersonation
            1, // TokenPrimary
            &mut dup,
        );
        CloseHandle(token);
        if ok == 0 {
            return Err(format!("DuplicateTokenEx: ошибка {}", GetLastError()));
        }
        let sid: u32 = session;
        if SetTokenInformation(dup, 12, &sid as *const u32 as *const c_void, 4) == 0 {
            let err = GetLastError();
            CloseHandle(dup);
            return Err(format!("SetTokenInformation: ошибка {err}"));
        }

        let desktop = wide("winsta0\\default");
        let mut si: StartupInfoW = std::mem::zeroed();
        si.cb = std::mem::size_of::<StartupInfoW>() as u32;
        si.lp_desktop = desktop.as_ptr() as *mut u16;
        si.dw_flags = 0x0000_0001; // STARTF_USESHOWWINDOW
        si.w_show_window = 1; // SW_SHOWNORMAL

        let mut pi: ProcessInformationW = std::mem::zeroed();
        let mut cmd_w = wide(&cmd);
        let created = CreateProcessAsUserW(
            dup,
            std::ptr::null(),
            cmd_w.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            0,
            std::ptr::null(),
            std::ptr::null(),
            &si,
            &mut pi,
        );
        let err = GetLastError();
        CloseHandle(dup);
        if created == 0 {
            return Err(format!("CreateProcessAsUser: ошибка {err}"));
        }
        CloseHandle(pi.h_thread);
        CloseHandle(pi.h_process);
        Ok(pi.dw_process_id)
    }
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

/// Зарегистрировать службу (автозапуск при загрузке), запустить её и убрать
/// задачу автозапуска из Планировщика (чтобы не было двух агентов).
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

/// Удалить задачу автозапуска «Rat Next» (при переходе на службу).
fn remove_autostart_task() {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("schtasks");
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let _ = cmd.args(["/Delete", "/F", "/TN", "Rat Next"]).output();
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
    log_line("служба удалена");
    Ok(())
}

/// Состояние службы для интерфейса.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Служба не зарегистрирована.
    NotInstalled,
    /// Зарегистрирована, но остановлена.
    Stopped,
    /// Работает.
    Running,
    /// Запускается или останавливается.
    Pending,
}

/// Операция со службой из интерфейса.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Install,
    Start,
    Stop,
    Uninstall,
}

/// Узнать состояние службы (только чтение, прав администратора не требует).
pub fn status() -> State {
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return State::NotInstalled;
        }
        let name = wide(SERVICE_NAME);
        let service = OpenServiceW(scm, name.as_ptr(), SERVICE_QUERY_STATUS);
        if service.is_null() {
            CloseServiceHandle(scm);
            return State::NotInstalled;
        }
        let mut st = ServiceStatus {
            service_type: 0,
            current_state: 0,
            controls_accepted: 0,
            win32_exit_code: 0,
            service_specific_exit_code: 0,
            checkpoint: 0,
            wait_hint: 0,
        };
        let ok = QueryServiceStatus(service, &mut st);
        CloseServiceHandle(service);
        CloseServiceHandle(scm);
        if ok == 0 {
            return State::NotInstalled;
        }
        match st.current_state {
            SERVICE_RUNNING => State::Running,
            SERVICE_STOPPED => State::Stopped,
            _ => State::Pending,
        }
    }
}

/// Запустить установленную службу.
pub fn start() -> Result<(), String> {
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_ALL_ACCESS);
        if scm.is_null() {
            return Err(format!(
                "OpenSCManager: ошибка {} (нужны права администратора)",
                GetLastError()
            ));
        }
        let name = wide(SERVICE_NAME);
        let service = OpenServiceW(scm, name.as_ptr(), SERVICE_START | SERVICE_QUERY_STATUS);
        if service.is_null() {
            let err = GetLastError();
            CloseServiceHandle(scm);
            if err == ERROR_SERVICE_DOES_NOT_EXIST {
                return Err("служба не установлена (сначала установите)".to_string());
            }
            return Err(format!("OpenService: ошибка {err}"));
        }
        if StartServiceW(service, 0, std::ptr::null()) == 0 {
            let err = GetLastError();
            CloseServiceHandle(service);
            CloseServiceHandle(scm);
            if err != ERROR_SERVICE_ALREADY_RUNNING {
                return Err(format!("StartService: ошибка {err}"));
            }
            return Ok(());
        }
        CloseServiceHandle(service);
        CloseServiceHandle(scm);
    }
    log_line("служба запущена (из интерфейса)");
    Ok(())
}

/// Остановить службу.
pub fn stop() -> Result<(), String> {
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_ALL_ACCESS);
        if scm.is_null() {
            return Err(format!(
                "OpenSCManager: ошибка {} (нужны права администратора)",
                GetLastError()
            ));
        }
        let name = wide(SERVICE_NAME);
        let service = OpenServiceW(scm, name.as_ptr(), SERVICE_STOP | SERVICE_QUERY_STATUS);
        if service.is_null() {
            let err = GetLastError();
            CloseServiceHandle(scm);
            if err == ERROR_SERVICE_DOES_NOT_EXIST {
                return Err("служба не установлена".to_string());
            }
            return Err(format!("OpenService: ошибка {err}"));
        }
        let mut st = ServiceStatus {
            service_type: 0,
            current_state: 0,
            controls_accepted: 0,
            win32_exit_code: 0,
            service_specific_exit_code: 0,
            checkpoint: 0,
            wait_hint: 0,
        };
        if ControlService(service, SERVICE_CONTROL_STOP, &mut st) == 0 {
            let err = GetLastError();
            CloseServiceHandle(service);
            CloseServiceHandle(scm);
            if err == ERROR_SERVICE_NOT_ACTIVE {
                return Ok(());
            }
            return Err(format!("ControlService: ошибка {err}"));
        }
        for _ in 0..40 {
            if QueryServiceStatus(service, &mut st) == 0 || st.current_state == SERVICE_STOPPED {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        CloseServiceHandle(service);
        CloseServiceHandle(scm);
    }
    log_line("служба остановлена (из интерфейса)");
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
    fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
    fn DuplicateTokenEx(
        existing: *mut c_void,
        access: u32,
        attrs: *mut c_void,
        impersonation_level: u32,
        token_type: u32,
        new_token: *mut *mut c_void,
    ) -> i32;
    fn SetTokenInformation(token: *mut c_void, class: u32, info: *const c_void, len: u32) -> i32;
    #[allow(clippy::too_many_arguments)]
    fn CreateProcessAsUserW(
        token: *mut c_void,
        app_name: *const u16,
        command_line: *mut u16,
        process_attrs: *mut c_void,
        thread_attrs: *mut c_void,
        inherit_handles: i32,
        flags: u32,
        environment: *const c_void,
        current_dir: *const u16,
        startup_info: *const StartupInfoW,
        process_info: *mut ProcessInformationW,
    ) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn GetCurrentProcess() -> *mut c_void;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn WaitForSingleObject(handle: *mut c_void, millis: u32) -> i32;
    fn TerminateProcess(handle: *mut c_void, code: u32) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn WTSGetActiveConsoleSessionId() -> u32;
}
