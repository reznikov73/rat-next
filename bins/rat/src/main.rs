//! `rat` — единое приложение Rat Next: морда с плитками, встроенный сервер
//! (приём подключений) и вьюер в одном окне.
//!
//! Пилот: машины адресуются как `IP:порт` (локальная сеть / Radmin VPN).
//! Позже (P4) адрес заменится на ID через relay/signaling — морда не изменится.
//!
//! Запуск:
//! ```text
//! rat.exe                                  # сервер стартует автоматически, порт 55140
//! rat.exe --port 55140
//! rat.exe --monitor 1                      # отдавать второй экран (индекс 1)
//! rat.exe --switch-after=4                 # тест: через 4 сек переключить монитор
//! rat.exe --autoconnect 192.168.1.50:55140
//! rat.exe --relay 91.x.x.x:55141 --device-id 123456           # интернет-режим
//! rat.exe --tray                              # стартовать сразу в трее (без окна)
//! rat.exe --install-service                   # служба: доступен с загрузки Windows (рекомендуется)
//! rat.exe --uninstall-service                 # снять службу
//! rat.exe --install-autostart                 # автозапуск при входе в Windows
//! rat.exe --remove-autostart                  # выключить автозапуск
//! rat.exe --smoke=15                       # авто-выход через 15 сек (тесты)
//! ```
#![windows_subsystem = "windows"]

#[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
struct Args {
    port: u16,
    monitor: u32,
    autoconnect: Option<String>,
    smoke: Option<f32>,
    switch_after: Option<f32>,
    relay: Option<String>,
    device_id: Option<String>,
    tray: bool,

    remove_autostart: bool,
    install_service: bool,
    uninstall_service: bool,
    start_service: bool,
    stop_service: bool,
    service: bool,
    service_helper: bool,
    probe_secure: bool,
    /// Проверка доставки ввода: подключиться к агенту по адресу и сверить курсор.
    input_check: Option<String>,
    /// Файловая проверка: адрес агента, дальше — команда (ls/get/put/…).
    files_check: Option<String>,
    file_cmd: Option<Vec<String>>,
    /// Авто-открытие файлового окна (тест/отладка): адрес агента.
    files_ui: Option<String>,
    /// Тест воркера файлов: адрес; далее три пути (исходная, удалённая, выгрузка).
    files_worker_test: Option<String>,
    /// Просьба о вежливом завершении работающего экземпляра (установщик).
    request_quit: bool,
}

#[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
fn parse_args() -> Args {
    let mut a = Args {
        port: 55140,
        monitor: 0,
        autoconnect: None,
        smoke: None,
        switch_after: None,
        relay: None,
        device_id: None,
        tray: false,

        remove_autostart: false,
        install_service: false,
        uninstall_service: false,
        start_service: false,
        stop_service: false,
        service: false,
        service_helper: false,
        probe_secure: false,
        input_check: None,
        files_check: None,
        file_cmd: None,
        files_ui: None,
        files_worker_test: None,
        request_quit: false,
    };
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        let cur = args[i].as_str();
        match cur {
            "--port" if i + 1 < args.len() => {
                a.port = args[i + 1].parse().unwrap_or(55140);
                i += 2;
            }
            "--monitor" if i + 1 < args.len() => {
                a.monitor = args[i + 1].parse().unwrap_or(0);
                i += 2;
            }
            "--autoconnect" if i + 1 < args.len() => {
                a.autoconnect = Some(args[i + 1].clone());
                i += 2;
            }

            "--remove-autostart" => {
                a.remove_autostart = true;
                i += 1;
            }
            "--tray" => {
                a.tray = true;
                i += 1;
            }
            "--install-service" => {
                a.install_service = true;
                i += 1;
            }
            "--uninstall-service" => {
                a.uninstall_service = true;
                i += 1;
            }
            "--start-service" => {
                a.start_service = true;
                i += 1;
            }
            "--stop-service" => {
                a.stop_service = true;
                i += 1;
            }
            "--service" => {
                a.service = true;
                i += 1;
            }
            "--probe-secure" => {
                a.probe_secure = true;
                i += 1;
            }
            "--input-check" => {
                a.input_check = Some("127.0.0.1:55140".to_string());
                i += 1;
            }
            s if s.starts_with("--input-check=") => {
                a.input_check = Some(s["--input-check=".len()..].to_string());
                i += 1;
            }
            s if s.starts_with("--files-check=") => {
                a.files_check = Some(s["--files-check=".len()..].to_string());
                a.file_cmd = Some(args[i + 1..].to_vec());
                i = args.len();
            }
            s if s.starts_with("--files-ui=") => {
                a.files_ui = Some(s["--files-ui=".len()..].to_string());
                i += 1;
            }
            s if s.starts_with("--files-worker-test=") => {
                a.files_worker_test = Some(s["--files-worker-test=".len()..].to_string());
                a.file_cmd = Some(args[i + 1..].to_vec());
                i = args.len();
            }

            "--request-quit" => {
                a.request_quit = true;
                i += 1;
            }
            "--service-helper" => {
                a.service_helper = true;
                i += 1;
            }
            "--relay" if i + 1 < args.len() => {
                a.relay = Some(args[i + 1].clone());
                i += 2;
            }
            "--device-id" if i + 1 < args.len() => {
                a.device_id = Some(args[i + 1].clone());
                i += 2;
            }
            s if s.starts_with("--switch-after=") => {
                a.switch_after = s["--switch-after=".len()..].parse().ok();
                i += 1;
            }
            s if s.starts_with("--smoke=") => {
                a.smoke = s["--smoke=".len()..].parse().ok();
                i += 1;
            }
            other => {
                eprintln!("неизвестный аргумент: {other}");
                i += 1;
            }
        }
    }
    a
}

#[cfg(not(feature = "ffmpeg"))]
fn main() {
    let _ = parse_args();
    eprintln!("rat: соберите с --features ffmpeg (пилотный кодек-путь)");
    std::process::exit(3);
}

#[cfg(feature = "ffmpeg")]
fn main() {
    let args = parse_args();
    // Сессионные потоки (агент/вьюер) пишут диагностику в тот же журнал.
    rat_session::log::set_logger(note);
    // Просьба о вежливом завершении работающего экземпляра (установщик,
    // скрипты обновления): доставляем сигнал и выходим, ничего не запуская.
    if args.request_quit {
        let delivered = rat_session::single_instance::quit_signal("app");
        note(&format!(
            "rat: запрос завершения — {}",
            if delivered {
                "доставлен"
            } else {
                "экземпляр не найден"
            }
        ));
        std::process::exit(0);
    }
    if args.install_service {
        match service::install() {
            Ok(()) => {
                note("service: установлена служба «RatNext» (задача автозапуска убрана)");
                println!("Служба установлена: агент будет доступен с момента загрузки Windows.");
                std::process::exit(0);
            }
            Err(e) => {
                note(&format!("service: ошибка установки: {e}"));
                eprintln!("service: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.uninstall_service {
        match service::uninstall() {
            Ok(()) => {
                note("service: служба удалена");
                println!("Служба удалена.");
                std::process::exit(0);
            }
            Err(e) => {
                note(&format!("service: ошибка удаления: {e}"));
                eprintln!("service: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.start_service {
        match service::start() {
            Ok(()) => {
                note("service: служба запущена (из интерфейса)");
                println!("Служба запущена.");
                std::process::exit(0);
            }
            Err(e) => {
                note(&format!("service: ошибка запуска: {e}"));
                eprintln!("service: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.stop_service {
        match service::stop() {
            Ok(()) => {
                note("service: служба остановлена (из интерфейса)");
                println!("Служба остановлена.");
                std::process::exit(0);
            }
            Err(e) => {
                note(&format!("service: ошибка остановки: {e}"));
                eprintln!("service: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.probe_secure {
        // Диагностика защищённого стола: имя активного стола + пробный снимок.
        let name = rat_session::secure::active_desktop_name();
        note(&format!("probe-secure: активный стол = {name:?}"));
        match rat_session::secure::SecureCapture::attach() {
            Ok(mut sc) => {
                let (w, h) = sc.size();
                let mut frame: Vec<u8> = Vec::new();
                let res = sc.capture(|bgra, _pitch| {
                    frame = bgra.to_vec();
                });
                note(&format!("probe-secure: захват {w}x{h}: {res:?}"));
                if matches!(res, Ok(true))
                    && let Ok(exe) = std::env::current_exe()
                {
                    let bmp = exe.with_file_name("probe-secure.bmp");
                    write_bmp(&bmp, w, h, &frame);
                    note(&format!("probe-secure: снимок сохранён: {}", bmp.display()));
                }
            }
            Err(e) => note(&format!("probe-secure: ошибка: {e}")),
        }
        std::process::exit(0);
    }
    if let Some(addr) = args.input_check.as_deref() {
        // Диагностика доставки ввода: движения мыши и сверка курсора.
        match rat_session::inputcheck::run(addr) {
            Ok(report) => note(&report),
            Err(e) => note(&format!("ввод-проверка: ошибка: {e}")),
        }
        std::process::exit(0);
    }
    if let Some(addr) = args.files_check.as_deref() {
        // Файловая проверка: команда в аргументах (ls/drives/get/put/mkdir/rm/mv).
        let cmd_args = args.file_cmd.clone().unwrap_or_default();
        match rat_session::files::parse_cmd(&cmd_args) {
            Ok(cmd) => match rat_session::files::client_run(addr, &cmd) {
                Ok(report) => {
                    println!("{report}");
                }
                Err(e) => {
                    println!("файлы: ошибка: {e}");
                    std::process::exit(1);
                }
            },
            Err(e) => {
                println!("файлы: {e}");
                std::process::exit(2);
            }
        }
        std::process::exit(0);
    }
    if let Some(addr) = args.files_worker_test.as_deref() {
        let rest = args.file_cmd.clone().unwrap_or_default();
        if rest.len() != 3 {
            println!("использование: --files-worker-test=АДРЕС <исходная> <удалённая> <выгрузка>");
            std::process::exit(2);
        }
        let target = rat_session::files_worker::Target::Tcp(addr.to_string());
        match rat_session::files_worker::selftest(target, &rest[0], &rest[1], &rest[2]) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                println!("ошибка: {e}");
                std::process::exit(1);
            }
        }
        std::process::exit(0);
    }
    if args.service {
        if let Err(e) = service::run_service() {
            note(&format!("service: {e}"));
            std::process::exit(1);
        }
        return;
    }
    // Задача-автозапуск «Rat Next» больше не используется — убираем устаревшую,
    // если она осталась с прошлых версий.
    if autostart::is_installed() {
        let _ = autostart::remove();
        note("autostart: устаревшая задача «Rat Next» удалена");
    }
    if args.remove_autostart {
        match autostart::remove() {
            Ok(()) => {
                println!("Автозапуск выключен.");
                note("autostart: удалён");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("autostart: {e}");
                note(&format!("autostart: ошибка удаления: {e}"));
                std::process::exit(1);
            }
        }
    }
    if !rat_session::single_instance::try_acquire("app") {
        if args.service_helper {
            // Помощник службы не выходит, а ждёт, пока закроется чужое окно:
            // служба остаётся спокойной и сразу подхватывает управление.
            note("rat: хелпер: жду освобождения (уже работает другое окно)");
            let mut steps: u32 = 0;
            loop {
                if rat_session::single_instance::try_acquire("app") {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
                steps += 1;
                if steps.is_multiple_of(20)
                    && matches!(
                        service::status(),
                        service::State::NotInstalled | service::State::Stopped
                    )
                {
                    note("rat: хелпер: служба остановлена — завершаюсь");
                    std::process::exit(0);
                }
            }
        } else {
            if args.smoke.is_none() {
                // Повторный запуск — просьба к работающему экземпляру показать окно
                // (то же, что клик по значку в трее; работает и когда значка нет).
                if rat_session::single_instance::show_signal("app") {
                    note("rat: повторный запуск — отправлен запрос «показать окно»");
                    std::process::exit(0);
                }
                rat_session::single_instance::warn(
                    "Rat Next",
                    "Rat Next уже запущен.\nОкно можно открыть из значка в трее.",
                );
            }
            note("rat: экземпляр уже запущен (двойной запуск заблокирован)");
            std::process::exit(2);
        }
    }
    note(&format!(
        "rat: одиночный запуск: mutex создан, имя={}",
        rat_session::single_instance::mutex_name_for_log("app")
    ));
    if let Err(e) = ui::run(args) {
        note(&format!("rat: {e}"));
        std::process::exit(1);
    }
}

// ================== журнал ==================

/// Сохранить снимок BGRA (сверху вниз) в BMP-файл (для диагностики).
#[cfg(feature = "ffmpeg")]
fn write_bmp(path: &std::path::Path, w: u32, h: u32, bgra: &[u8]) {
    use std::io::Write as _;
    let row = (w as usize) * 4;
    if bgra.len() < row * (h as usize) {
        return;
    }
    let data_size = row * (h as usize);
    let mut hdr = Vec::with_capacity(54);
    hdr.extend_from_slice(b"BM");
    hdr.extend_from_slice(&((54 + data_size) as u32).to_le_bytes());
    hdr.extend_from_slice(&[0u8; 4]);
    hdr.extend_from_slice(&54u32.to_le_bytes());
    hdr.extend_from_slice(&40u32.to_le_bytes());
    hdr.extend_from_slice(&(w as i32).to_le_bytes());
    hdr.extend_from_slice(&(h as i32).to_le_bytes());
    hdr.extend_from_slice(&1u16.to_le_bytes());
    hdr.extend_from_slice(&32u16.to_le_bytes());
    hdr.extend_from_slice(&0u32.to_le_bytes());
    hdr.extend_from_slice(&(data_size as u32).to_le_bytes());
    hdr.extend_from_slice(&2835u32.to_le_bytes());
    hdr.extend_from_slice(&2835u32.to_le_bytes());
    hdr.extend_from_slice(&0u32.to_le_bytes());
    hdr.extend_from_slice(&0u32.to_le_bytes());
    if let Ok(mut f) = std::fs::File::create(path) {
        let _ = f.write_all(&hdr);
        for y in (0..h as usize).rev() {
            let _ = f.write_all(&bgra[y * row..(y + 1) * row]);
        }
    }
}

/// Записать строку в `rat.log` рядом с exe (переживает отсутствие консоли).
#[cfg(feature = "ffmpeg")]
fn log_line(s: &str) {
    use std::io::Write as _;
    let path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("rat.log")));
    if let Some(p) = path
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p)
    {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "{t} {s}");
    }
}

/// Лог: в stdout (если он есть) и в `rat.log`.
#[cfg(feature = "ffmpeg")]
fn note(s: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stdout(), "{s}");
    log_line(s);
}

// ================== значок приложения и трей ==================

/// Рисование иконки: тот же код использует `build.rs`, чтобы вложить `.ico` в exe.
#[cfg(feature = "ffmpeg")]
mod icon;

/// Значок в области уведомлений.
#[cfg(feature = "ffmpeg")]
mod elevate;
#[cfg(feature = "ffmpeg")]
mod service;

#[cfg(feature = "ffmpeg")]
mod tray;

#[cfg(feature = "ffmpeg")]
#[link(name = "user32")]
unsafe extern "system" {
    fn FindWindowW(class_name: *const u16, window_name: *const u16) -> *mut std::ffi::c_void;
}

/// Готов ли рабочий стол с панелью задач (поднят ли проводник). Пока нет —
/// значок в трее создавать нельзя: неудачная попытка до старта панели задач
/// оставляет служебное окно, которое при рассылке TaskbarCreated «оживает»
/// вторым значком — в трее появлялся дубликат «Rat Next — служба».
#[cfg(feature = "ffmpeg")]
fn shell_ready() -> bool {
    let class: Vec<u16> = "Shell_TrayWnd\0".encode_utf16().collect();
    unsafe { !FindWindowW(class.as_ptr(), std::ptr::null()).is_null() }
}

// ================== хранилище машин ==================

#[cfg(feature = "ffmpeg")]
mod store {
    use serde::{Deserialize, Serialize};
    use std::path::PathBuf;

    #[derive(Serialize, Deserialize, Clone)]
    pub struct Machine {
        pub name: String,
        pub addr: String,
        /// Unix-время последнего подключения.
        pub last_connected: Option<i64>,
    }

    pub fn path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("machines.json")))
            .unwrap_or_else(|| PathBuf::from("machines.json"))
    }

    pub fn load() -> Vec<Machine> {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(list: &[Machine]) {
        if let Ok(s) = serde_json::to_string_pretty(list) {
            let _ = std::fs::write(path(), s);
        }
    }

    pub const DEFAULT_RELAY: &str = "194.37.254.233:8889";

    #[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
    #[serde(rename_all = "snake_case")]
    pub enum ConnectionMode {
        #[default]
        Internet,
        Lan,
    }

    /// Настройки этого узла (device.json рядом с exe).
    #[derive(Serialize, Deserialize, Clone)]
    #[serde(default)]
    pub struct Device {
        pub id: String,
        pub relay: String,
        pub mode: ConnectionMode,
        /// Принимать входящие подключения без запроса подтверждения.
        pub auto_allow: bool,
        /// Прятать окно в трей вместо выхода по крестику.
        pub tray: bool,
    }

    impl Default for Device {
        fn default() -> Self {
            Self {
                id: String::new(),
                relay: DEFAULT_RELAY.to_string(),
                mode: ConnectionMode::Internet,
                auto_allow: false,
                tray: true,
            }
        }
    }

    pub fn device_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("device.json")))
            .unwrap_or_else(|| PathBuf::from("device.json"))
    }

    pub fn load_device() -> Device {
        let mut device = std::fs::read_to_string(device_path())
            .ok()
            .and_then(|s| serde_json::from_str::<Device>(&s).ok())
            .unwrap_or_default();
        device.relay = normalize_relay(&device.relay);
        // ID — детерминированно из отпечатка машины (не зависит от переустановок).
        // Этап разработки: сохранённый ID приоритета не имеет — при расхождении
        // файл обновляется. Отпечаток не собрался — оставляем как есть
        // (существующий ID или случайный, см. место генерации ниже).
        if let Some(id) = fingerprint_id()
            && device.id != id
        {
            device.id = id;
            save_device(&device);
        }
        device
    }

    pub fn save_device(d: &Device) {
        if let Ok(s) = serde_json::to_string_pretty(d) {
            let _ = std::fs::write(device_path(), s);
        }
    }

    /// ID из отпечатка машины: MachineGuid | BIOS | том | имя → FNV-1a → 6 цифр.
    /// `None`, если ни один источник не прочитался (тогда прежний путь генерации).
    pub fn fingerprint_id() -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(v) = reg_string(r"SOFTWARE\Microsoft\Cryptography", "MachineGuid") {
            parts.push(v);
        }
        let bios = r"HARDWARE\DESCRIPTION\System\BIOS";
        if let Some(v) = reg_string(bios, "SystemProductName") {
            parts.push(v);
        }
        if let Some(v) = reg_string(bios, "BaseBoardProduct") {
            parts.push(v);
        }
        let root = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        if let Some(v) = volume_serial(&format!("{root}\\")) {
            parts.push(v);
        }
        if let Some(v) = computer_name() {
            parts.push(v);
        }
        if parts.is_empty() {
            return None;
        }
        let hash = fnv1a64(parts.join("|").as_bytes());
        Some(format!("{:06}", hash % 1_000_000))
    }

    /// Оставить только строчные латинские буквы и цифры (стабильность отпечатка).
    fn sanitize(s: &str) -> String {
        s.chars()
            .filter(char::is_ascii_alphanumeric)
            .flat_map(|c| c.to_lowercase())
            .collect()
    }

    /// FNV-1a 64 бита — свой, фиксированный (не зависит от версии компилятора).
    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for b in bytes {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    /// Строковое значение из реестра (только чтение, прав администратора не требует).
    #[cfg(windows)]
    fn reg_string(subkey: &str, value: &str) -> Option<String> {
        unsafe {
            let sub = to_wide(subkey);
            let val = to_wide(value);
            let mut buf = [0u16; 256];
            let mut len = (buf.len() * 2) as u32;
            let status = RegGetValueW(
                0x8000_0002_usize as *mut std::ffi::c_void, // HKEY_LOCAL_MACHINE
                sub.as_ptr(),
                val.as_ptr(),
                0x0000_0002, // RRF_RT_REG_SZ
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                &mut len,
            );
            if status != 0 || len < 2 {
                return None;
            }
            let chars = ((len as usize) / 2).saturating_sub(1).min(buf.len());
            let s = sanitize(&String::from_utf16_lossy(&buf[..chars]));
            if s.is_empty() { None } else { Some(s) }
        }
    }

    #[cfg(not(windows))]
    fn reg_string(_subkey: &str, _value: &str) -> Option<String> {
        None
    }

    /// Серийный номер тома (например, системного) в нижнем hex.
    #[cfg(windows)]
    fn volume_serial(root: &str) -> Option<String> {
        let root_w = to_wide(root);
        let mut serial = 0u32;
        let ok = unsafe {
            GetVolumeInformationW(
                root_w.as_ptr(),
                std::ptr::null_mut(),
                0,
                &mut serial,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        if ok != 0 {
            Some(format!("{serial:08x}"))
        } else {
            None
        }
    }

    #[cfg(not(windows))]
    fn volume_serial(_root: &str) -> Option<String> {
        None
    }

    /// Имя машины.
    #[cfg(windows)]
    fn computer_name() -> Option<String> {
        let mut buf = [0u16; 64];
        let mut len = buf.len() as u32;
        let ok = unsafe { GetComputerNameW(buf.as_mut_ptr(), &mut len) };
        if ok == 0 || len == 0 {
            return None;
        }
        let s = sanitize(&String::from_utf16_lossy(&buf[..len as usize]));
        if s.is_empty() { None } else { Some(s) }
    }

    #[cfg(not(windows))]
    fn computer_name() -> Option<String> {
        None
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[cfg(windows)]
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            hkey: *mut std::ffi::c_void,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            value_type: *mut u32,
            data: *mut std::ffi::c_void,
            data_len: *mut u32,
        ) -> i32;
    }

    #[cfg(windows)]
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetVolumeInformationW(
            root: *const u16,
            volume_name: *mut u16,
            volume_name_len: u32,
            serial: *mut u32,
            max_component_len: *mut u32,
            fs_flags: *mut u32,
            fs_name: *mut u16,
            fs_name_len: u32,
        ) -> i32;
        fn GetComputerNameW(buffer: *mut u16, size: *mut u32) -> i32;
    }

    pub fn is_valid_id(id: &str) -> bool {
        id.len() == 6 && id.bytes().all(|byte| byte.is_ascii_digit())
    }

    pub fn gen_id() -> Result<String, std::io::Error> {
        let mut bytes = [0_u8; 4];
        fill_random(&mut bytes)?;
        let number = 100_000 + u32::from_le_bytes(bytes) % 900_000;
        Ok(format!("{number:06}"))
    }

    #[cfg(windows)]
    fn fill_random(bytes: &mut [u8]) -> Result<(), std::io::Error> {
        #[link(name = "bcrypt")]
        unsafe extern "system" {
            fn BCryptGenRandom(
                algorithm: *mut std::ffi::c_void,
                buffer: *mut u8,
                length: u32,
                flags: u32,
            ) -> i32;
        }
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                2,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(std::io::Error::other(format!("BCryptGenRandom: {status}")))
        }
    }

    #[cfg(not(windows))]
    fn fill_random(bytes: &mut [u8]) -> Result<(), std::io::Error> {
        use std::io::Read;
        std::fs::File::open("/dev/urandom")?.read_exact(bytes)
    }

    pub fn normalize_relay(raw: &str) -> String {
        let relay = raw.trim();
        if relay.is_empty() {
            return DEFAULT_RELAY.to_string();
        }
        let has_port = relay
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse::<u16>().ok())
            .is_some();
        if has_port || relay.contains(':') {
            relay.to_string()
        } else {
            format!("{relay}:8889")
        }
    }

    /// Нормализовать: IP → добавить порт по умолчанию; ID — как есть.
    pub fn normalize_addr(raw: &str) -> String {
        let s = raw.trim();
        if s.contains(':') {
            s.to_string()
        } else if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.') {
            format!("{s}:55140")
        } else {
            s.to_string()
        }
    }

    pub fn now_unix() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    #[allow(dead_code)]
    pub fn ago_text(ts: i64) -> String {
        let d = now_unix() - ts;
        if d < 60 {
            "только что".into()
        } else if d < 3600 {
            format!("{} мин назад", d / 60)
        } else if d < 86400 {
            format!("{} ч назад", d / 3600)
        } else {
            format!("{} дн назад", d / 86400)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn generated_id_has_six_digits() {
            let id = gen_id().unwrap();
            assert!(is_valid_id(&id));
        }

        #[test]
        fn fnv1a_known_vector() {
            // Известное значение FNV-1a 64 для "hello".
            assert_eq!(fnv1a64(b"hello"), 0xa430_d846_80aa_bd0b);
        }

        #[test]
        fn fingerprint_is_six_digits_when_available() {
            if let Some(id) = fingerprint_id() {
                assert!(is_valid_id(&id), "некорректный ID: {id}");
            }
        }

        #[test]
        fn relay_address_gets_default_port() {
            assert_eq!(normalize_relay("194.37.254.233"), "194.37.254.233:8889");
            assert_eq!(normalize_relay(""), DEFAULT_RELAY);
            assert_eq!(normalize_relay("relay.example:9000"), "relay.example:9000");
            assert_eq!(normalize_relay("relay.example:bad"), "relay.example:bad");
        }
    }
}

#[cfg(feature = "ffmpeg")]
mod notify {
    //! Системные сигналы (звук) — без внешних зависимостей.

    /// Короткий системный сигнал (запрос подключения).
    pub fn beep() {
        #[cfg(windows)]
        unsafe {
            MessageBeep(0x0000_0040); // MB_ICONASTERISK
        }
    }

    #[cfg(windows)]
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBeep(u_type: u32) -> i32;
    }
}

#[cfg(feature = "ffmpeg")]
mod autostart {
    //! Автозапуск при входе пользователя через Планировщик задач (`schtasks`).

    const TASK: &str = "Rat Next";

    /// `schtasks` без мелькания консольного окна.
    fn schtasks() -> std::process::Command {
        let mut cmd = std::process::Command::new("schtasks");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        cmd
    }

    /// Установлена ли задача автозапуска.
    pub fn is_installed() -> bool {
        schtasks()
            .args(["/Query", "/TN", TASK])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Удалить задачу автозапуска.
    pub fn remove() -> Result<(), String> {
        let out = schtasks()
            .args(["/Delete", "/F", "/TN", TASK])
            .output()
            .map_err(|e| format!("schtasks: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let err = if err.is_empty() {
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            } else {
                err
            };
            Err(err)
        }
    }
}

#[cfg(feature = "ffmpeg")]
mod approval {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Sender, SyncSender};
    use std::time::{Duration, Instant};

    #[derive(Clone, Copy)]
    pub enum Source {
        Lan,
        Internet,
    }

    pub struct Request {
        pub source: Source,
        pub expires_at: Instant,
        pub reply: SyncSender<bool>,
    }

    /// Настройка «Подключать без подтверждения» из окна настроек.
    static AUTO_ALLOW: AtomicBool = AtomicBool::new(false);

    pub fn set_auto_allow(on: bool) {
        AUTO_ALLOW.store(on, Ordering::Relaxed);
    }

    pub fn request(requests: &Sender<Request>, source: Source, running: &AtomicBool) -> bool {
        if AUTO_ALLOW.load(Ordering::Relaxed) {
            crate::note(&format!(
                "[guard] подключение разрешено без подтверждения ({})",
                match source {
                    Source::Lan => "локальная сеть",
                    Source::Internet => "интернет",
                }
            ));
            return true;
        }
        let (reply, decision) = mpsc::sync_channel(1);
        let expires_at = Instant::now() + Duration::from_secs(60);
        if requests
            .send(Request {
                source,
                expires_at,
                reply,
            })
            .is_err()
        {
            return false;
        }
        loop {
            if !running.load(Ordering::Relaxed) {
                return false;
            }
            let remaining = expires_at.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            match decision.recv_timeout(remaining.min(Duration::from_millis(200))) {
                Ok(allowed) => return allowed,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return false,
            }
        }
    }
}

// ================== встроенный сервер ==================

#[cfg(feature = "ffmpeg")]
mod server {
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use std::sync::mpsc::Sender;

    use rat_session::agent::{AgentConfig, spawn_agent};
    use rat_session::link::tcp_accept;
    use rat_session::stats::{SessionStats, TimeBase};

    pub struct ServerHandle {
        pub port: u16,
        running: Arc<AtomicBool>,
        stop_current: Arc<Mutex<Option<Arc<AtomicBool>>>>,
        thread: Option<JoinHandle<()>>,
        incoming_requests: Sender<crate::approval::Request>,
        pub sessions: Arc<AtomicU64>,
        /// Какой монитор отдавать (индекс DXGI; применяется к новым сессиям).
        pub monitor: Arc<AtomicU32>,
        pub last_err: Arc<Mutex<Option<String>>>,
    }

    impl ServerHandle {
        pub fn new(incoming_requests: Sender<crate::approval::Request>) -> Self {
            Self {
                port: 55140,
                running: Arc::new(AtomicBool::new(false)),
                stop_current: Arc::new(Mutex::new(None)),
                thread: None,
                incoming_requests,
                sessions: Arc::new(AtomicU64::new(0)),
                monitor: Arc::new(AtomicU32::new(0)),
                last_err: Arc::new(Mutex::new(None)),
            }
        }

        pub fn is_running(&self) -> bool {
            self.running.load(Ordering::Relaxed)
        }

        pub fn start(&mut self, port: u16) -> Result<(), String> {
            if self.is_running() {
                return Ok(());
            }
            let listener = TcpListener::bind(("0.0.0.0", port))
                .map_err(|e| format!("не удалось занять порт {port}: {e}"))?;
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
            *self.last_err.lock().unwrap() = None;
            self.running.store(true, Ordering::Relaxed);
            self.port = port;
            let running = self.running.clone();
            let stop_current = self.stop_current.clone();
            let incoming_requests = self.incoming_requests.clone();
            let sessions = self.sessions.clone();
            let monitor = self.monitor.clone();
            let last_err = self.last_err.clone();
            crate::note(&format!("[server] слушаю 0.0.0.0:{port}"));
            self.thread = Some(std::thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    match tcp_accept(&listener) {
                        Ok((link, closer)) => {
                            let approved = crate::approval::request(
                                &incoming_requests,
                                crate::approval::Source::Lan,
                                &running,
                            );
                            if !approved {
                                closer.close();
                                continue;
                            }
                            crate::note("[server] клиент подключился");
                            sessions.fetch_add(1, Ordering::Relaxed);
                            let base = TimeBase(Instant::now());
                            let stop = Arc::new(AtomicBool::new(false));
                            *stop_current.lock().unwrap() = Some(stop.clone());
                            let stats = Arc::new(Mutex::new(SessionStats::default()));
                            let cfg = AgentConfig {
                                monitor: monitor.load(Ordering::Relaxed),
                                ..AgentConfig::default()
                            };
                            match spawn_agent(base, stop.clone(), stats.clone(), link, cfg) {
                                Ok(handles) => {
                                    for h in handles {
                                        let _ = h.join();
                                    }
                                    closer.close();
                                    let st = stats.lock().unwrap();
                                    crate::note(&format!(
                                        "[server] сессия завершена\n{}",
                                        st.summary()
                                    ));
                                }
                                Err(e) => {
                                    closer.close();
                                    crate::note(&format!("[server] ошибка сессии: {e}"));
                                    *last_err.lock().unwrap() = Some(e);
                                }
                            }
                            *stop_current.lock().unwrap() = None;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(150));
                        }
                        Err(e) => {
                            *last_err.lock().unwrap() = Some(format!("accept: {e}"));
                            std::thread::sleep(Duration::from_millis(500));
                        }
                    }
                }
                crate::note("[server] остановлен");
            }));
            Ok(())
        }

        pub fn stop(&mut self) {
            self.running.store(false, Ordering::Relaxed);
            if let Some(s) = self.stop_current.lock().unwrap().take() {
                s.store(true, Ordering::Relaxed);
            }
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
}

// ================== интернет-режим (relay-клиент) ==================

#[cfg(feature = "ffmpeg")]
mod internet {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use rat_session::agent::{AgentConfig, spawn_agent};
    use rat_session::link::{TcpCloser, relay_register, relay_wait_paired};
    use rat_session::stats::{SessionStats, TimeBase};

    pub struct InternetHandle {
        running: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
        incoming_requests: Sender<crate::approval::Request>,
        pub status: Arc<Mutex<String>>,
        pub sessions: Arc<AtomicU64>,
        abort: Arc<Mutex<Option<TcpCloser>>>,
        stop_current: Arc<Mutex<Option<Arc<AtomicBool>>>>,
        monitor: Arc<std::sync::atomic::AtomicU32>,
    }

    impl InternetHandle {
        pub fn new(
            monitor: Arc<std::sync::atomic::AtomicU32>,
            incoming_requests: Sender<crate::approval::Request>,
        ) -> Self {
            Self {
                running: Arc::new(AtomicBool::new(false)),
                thread: None,
                incoming_requests,
                status: Arc::new(Mutex::new("выключен".to_string())),
                sessions: Arc::new(AtomicU64::new(0)),
                abort: Arc::new(Mutex::new(None)),
                stop_current: Arc::new(Mutex::new(None)),
                monitor,
            }
        }

        pub fn is_running(&self) -> bool {
            self.running.load(Ordering::Relaxed)
        }

        pub fn start(&mut self, relay: String, id: String, name: String) {
            if self.is_running() {
                return;
            }
            self.running.store(true, Ordering::Relaxed);
            let running = self.running.clone();
            let status = self.status.clone();
            let sessions = self.sessions.clone();
            let abort = self.abort.clone();
            let stop_current = self.stop_current.clone();
            let incoming_requests = self.incoming_requests.clone();
            let monitor = self.monitor.clone();
            self.thread = Some(std::thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    crate::note(&format!("[internet] подключаюсь: {relay} ({id})"));
                    *status.lock().unwrap() = "подключаюсь к серверу…".to_string();
                    let reg = relay_register(relay.as_str(), &id, &name);
                    let (stream, aborter) = match reg {
                        Ok(v) => v,
                        Err(e) => {
                            crate::note(&format!("[internet] ошибка подключения: {e}"));
                            *status.lock().unwrap() = format!("ошибка: {e}");
                            sleep_checked(&running, 5000);
                            continue;
                        }
                    };
                    crate::note(&format!("[internet] зарегистрирован: {id}"));
                    *abort.lock().unwrap() = Some(aborter);
                    *status.lock().unwrap() =
                        "ожидаю подключения (сообщите ID оператору)".to_string();
                    match relay_wait_paired(stream) {
                        Err(e) => {
                            *abort.lock().unwrap() = None;
                            if running.load(Ordering::Relaxed) {
                                crate::note(&format!("[internet] соединение закрыто: {e}"));
                                *status.lock().unwrap() = format!("соединение закрыто: {e}");
                                sleep_checked(&running, 3000);
                            }
                            continue;
                        }
                        Ok((link, closer)) => {
                            *abort.lock().unwrap() = None;
                            *status.lock().unwrap() =
                                "ожидаю разрешения на подключение".to_string();
                            let approved = crate::approval::request(
                                &incoming_requests,
                                crate::approval::Source::Internet,
                                &running,
                            );
                            if !approved {
                                closer.close();
                                *status.lock().unwrap() =
                                    "подключение отклонено или истёк запрос".to_string();
                                continue;
                            }
                            *status.lock().unwrap() = "сессия (через сервер)".to_string();
                            sessions.fetch_add(1, Ordering::Relaxed);
                            let mon = monitor.load(Ordering::Relaxed);
                            serve_session(link, closer, &stop_current, mon);
                        }
                    }
                }
                *status.lock().unwrap() = "выключен".to_string();
            }));
        }

        pub fn stop(&mut self) {
            self.running.store(false, Ordering::Relaxed);
            if let Some(a) = self.abort.lock().unwrap().take() {
                a.close();
            }
            if let Some(s) = self.stop_current.lock().unwrap().take() {
                s.store(true, Ordering::Relaxed);
            }
            if let Some(t) = self.thread.take() {
                std::thread::spawn(move || {
                    let _ = t.join();
                });
            }
        }
    }

    fn sleep_checked(running: &AtomicBool, ms: u64) {
        let mut left = ms;
        while left > 0 && running.load(Ordering::Relaxed) {
            let step = left.min(250);
            std::thread::sleep(Duration::from_millis(step));
            left -= step;
        }
    }

    /// Обслужить одну сессию поверх готового линка (relay-путь).
    fn serve_session(
        link: rat_session::link::Duplex,
        closer: TcpCloser,
        stop_current: &Arc<Mutex<Option<Arc<AtomicBool>>>>,
        monitor: u32,
    ) {
        let base = TimeBase(Instant::now());
        let stop = Arc::new(AtomicBool::new(false));
        *stop_current.lock().unwrap() = Some(stop.clone());
        let stats = Arc::new(Mutex::new(SessionStats::default()));
        let cfg = AgentConfig {
            monitor,
            ..AgentConfig::default()
        };
        match spawn_agent(base, stop.clone(), stats.clone(), link, cfg) {
            Ok(handles) => {
                for h in handles {
                    let _ = h.join();
                }
                closer.close();
                let st = stats.lock().unwrap();
                crate::note(&format!("[internet] сессия завершена\n{}", st.summary()));
            }
            Err(e) => {
                closer.close();
                crate::note(&format!("[internet] ошибка сессии: {e}"));
            }
        }
        *stop_current.lock().unwrap() = None;
    }
}

// ================== приложение (egui) ==================

#[cfg(feature = "ffmpeg")]
mod ui {
    use serde::{Deserialize, Serialize};
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::net::ToSocketAddrs;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, TryRecvError, channel, sync_channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use eframe::egui;

    use rat_session::input::{self, CursorState, InputEvent, SessionInfo};
    use rat_session::link::{TcpCloser, tcp_connect};
    use rat_session::stats::{SessionStats, TimeBase};
    use rat_session::viewer::{FrameOut, ViewerShared, spawn_viewer};

    use super::Args;
    use super::approval::{Request, Source};
    use super::internet::InternetHandle;
    use super::server::ServerHandle;
    use super::store::{
        ConnectionMode, Device, Machine, gen_id, is_valid_id, load, load_device, normalize_addr,
        normalize_relay, now_unix, save, save_device,
    };

    /// Светлые карточки-плитки — как в согласованном макете.
    const CARD_BG: egui::Color32 = egui::Color32::from_rgb(252, 252, 253);
    const CARD_BORDER: egui::Color32 = egui::Color32::from_rgb(219, 223, 229);
    const CARD_BORDER_HOVER: egui::Color32 = egui::Color32::from_rgb(59, 130, 246);
    /// Плитка «+ Добавить машину», текст плиток и кнопка «Подключиться».
    const ADD_CARD_BG: egui::Color32 = egui::Color32::from_rgb(248, 249, 251);
    const TILE_TEXT: egui::Color32 = egui::Color32::from_rgb(31, 35, 40);
    const TILE_OFF_DOT: egui::Color32 = egui::Color32::from_rgb(197, 202, 210);
    const BTN_BLUE: egui::Color32 = egui::Color32::from_rgb(59, 130, 246);
    const BTN_BLUE_DIM: egui::Color32 = egui::Color32::from_rgb(126, 165, 244);
    const ON: egui::Color32 = egui::Color32::from_rgb(76, 175, 80); // цвет 
    const OFF: egui::Color32 = egui::Color32::from_rgb(244, 67, 54);
    const DIM: egui::Color32 = egui::Color32::from_rgb(150, 158, 170);
    const DIM2: egui::Color32 = egui::Color32::from_rgb(120, 128, 140);

    /// Имя семейства с настоящим жирным шрифтом для номера ID.
    const ID_BOLD: &str = "id-bold";

    /// Подключает системный жирный шрифт для номера ID (`strong()` в egui
    /// меняет только цвет — реальной жирности в комплекте нет).
    fn install_ui_fonts(ctx: &egui::Context) {
        let candidates = [
            r"C:\Windows\Fonts\segoeuib.ttf",
            r"C:\Windows\Fonts\seguisb.ttf",
            r"C:\Windows\Fonts\arialbd.ttf",
        ];
        let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else {
            return;
        };
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            ID_BOLD.to_owned(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        let mut family = vec![ID_BOLD.to_owned()];
        // Запасной вариант — шрифт по умолчанию.
        if let Some(default_font) = fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .and_then(|f| f.first().cloned())
        {
            family.push(default_font);
        }
        fonts
            .families
            .insert(egui::FontFamily::Name(ID_BOLD.into()), family);
        ctx.set_fonts(fonts);
    }

    pub fn run(args: Args) -> Result<(), String> {
        if std::env::var_os("WGPU_BACKEND").is_none() {
            // SAFETY: до инициализации GUI, однопоточно.
            unsafe { std::env::set_var("WGPU_BACKEND", "dx12") };
        }
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1020.0, 700.0])
                .with_icon(window_icon()),
            ..Default::default()
        };
        // Слушатель повторного запуска: второй rat.exe просит показать окно.
        let repaint_slot: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let show_request = {
            let slot = repaint_slot.clone();
            rat_session::single_instance::show_listener("app", move || {
                if let Some(ctx) = slot.lock().ok().and_then(|c| c.clone()) {
                    ctx.request_repaint();
                }
            })
        };
        // Слушатель вежливого завершения: установщик (или скрипт обновления)
        // просит экземпляр выйти — штатный выход сам уберёт значок трея.
        let quit_request = {
            let slot = repaint_slot.clone();
            rat_session::single_instance::quit_listener("app", move || {
                if let Some(ctx) = slot.lock().ok().and_then(|c| c.clone()) {
                    ctx.request_repaint();
                }
            })
        };
        eframe::run_native(
            "Rat Next",
            options,
            Box::new(move |cc| {
                install_ui_fonts(&cc.egui_ctx);
                Ok(Box::new(RatApp::new(
                    args,
                    show_request,
                    quit_request,
                    repaint_slot,
                )))
            }),
        )
        .map_err(|e| e.to_string())
    }

    /// Иконка окна и панели задач — тот же рисунок, что у трея и у exe.
    fn window_icon() -> egui::IconData {
        const SIDE: u32 = 64;
        egui::IconData {
            rgba: crate::icon::rgba(SIDE),
            width: SIDE,
            height: SIDE,
        }
    }

    type ConnectMsg = Result<ViewerSession, String>;

    // ---------------- файловый менеджер ----------------

    type FsDirEntry = rat_session::files::DirEntry;
    type FsCmd = rat_session::files_worker::WCmd;
    type FsEvt = rat_session::files_worker::WEvt;
    type FsAction = rat_session::files_worker::ConflictAction;

    /// Элемент очереди передач (отображение).
    struct UiJob {
        id: u64,
        up: bool,
        name: String,
        total: u64,
        done: u64,
        status: JobStatusUi,
        speed: f32,
        last_done: u64,
        last_t: Instant,
        note: String,
        is_dir: bool,
    }

    enum JobStatusUi {
        Queued,
        Active,
        Done,
        Skipped,
        Failed(String),
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Side {
        Local,
        Remote,
    }

    struct PaneState {
        path: String,
        entries: Vec<FsDirEntry>,
        /// Множественное выделение (индексы записей).
        sel: BTreeSet<usize>,
        /// «Якорь» для Shift-диапазона.
        anchor: Option<usize>,
        loading: bool,
        err: Option<String>,
        /// Свободно/всего на томе текущего пути.
        free: Option<(u64, u64)>,
    }

    impl PaneState {
        fn new(path: String) -> Self {
            Self {
                path,
                entries: Vec::new(),
                sel: BTreeSet::new(),
                anchor: None,
                loading: false,
                err: None,
                free: None,
            }
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum PromptKind {
        NewFolder,
        Rename,
    }

    struct PromptState {
        kind: PromptKind,
        side: Side,
        text: String,
        target: String,
    }

    struct ConfirmState {
        side: Side,
        /// Что удаляем: (полный путь, имя, это папка).
        items: Vec<(String, String, bool)>,
    }

    struct ConflictState {
        id: u64,
        path: String,
        up: bool,
        name: String,
        choice: u8,
        apply_all: bool,
        new_name: String,
    }

    /// Переносимый набор: панель-источник и выбранные объекты
    /// (имя, папка?, размер).
    #[derive(Clone)]
    struct DragPayload {
        side: Side,
        items: Vec<(String, bool, u64)>,
    }

    /// Состояние экрана «Файлы».
    struct FilesState {
        name: String,
        addr: String,
        tx: std::sync::mpsc::Sender<FsCmd>,
        rx: std::sync::mpsc::Receiver<FsEvt>,
        connected: bool,
        failed: Option<String>,
        side: Side,
        local: PaneState,
        remote: PaneState,
        remote_drives: Vec<String>,
        jobs: Vec<UiJob>,
        next_job: u64,
        conflict: Option<ConflictState>,
        conflict_all: Option<bool>,
        prompt: Option<PromptState>,
        confirm: Option<ConfirmState>,
        status: Option<String>,
        status_t: Instant,
        want_close: bool,
        drag_src: Option<DragPayload>,
        /// Пробуем восстановить сохранённый путь удалённой панели:
        /// если он не открылся — перейдём на первый диск.
        remote_restore: bool,
    }

    impl FilesState {
        fn open(name: String, addr: String, target: rat_session::files_worker::Target) -> Self {
            let (tx, rx) = rat_session::files_worker::spawn(target);
            // Пути панелей помним между открытиями окна и перезапусками (ui.json).
            let prefs = load_ui_prefs();
            let local_saved = prefs.files_local.clone();
            let local_path =
                if !local_saved.is_empty() && std::path::Path::new(&local_saved).is_dir() {
                    local_saved
                } else {
                    local_home_dir()
                };
            let remote_saved = prefs.files_remote.get(&addr).cloned();
            let remote_path = remote_saved.clone().unwrap_or_else(|| "C:\\".to_string());
            let mut s = Self {
                name,
                addr,
                tx,
                rx,
                connected: false,
                failed: None,
                side: Side::Local,
                local: PaneState::new(local_path),
                remote: PaneState::new(remote_path),
                remote_drives: Vec::new(),
                jobs: Vec::new(),
                next_job: 1,
                conflict: None,
                conflict_all: None,
                prompt: None,
                confirm: None,
                status: None,
                status_t: Instant::now() - Duration::from_secs(10),
                want_close: false,
                drag_src: None,
                remote_restore: remote_saved.is_some(),
            };
            s.reload(Side::Local);
            s
        }

        fn shutdown(&mut self) {
            let _ = self.tx.send(FsCmd::Quit);
        }

        fn note(&mut self, s: String) {
            self.status = Some(s);
            self.status_t = Instant::now();
        }

        fn reload(&mut self, side: Side) {
            match side {
                Side::Local => {
                    let p = &mut self.local;
                    match list_dir(&p.path) {
                        Ok(e) => {
                            p.entries = e;
                            p.sel.clear();
                            p.anchor = None;
                            p.err = None;
                        }
                        Err(e) => p.err = Some(e),
                    }
                    p.free = rat_session::files::drive_space(&p.path);
                }
                Side::Remote => {
                    self.remote.loading = true;
                    self.remote.err = None;
                    let _ = self.tx.send(FsCmd::List {
                        path: self.remote.path.clone(),
                    });
                    let _ = self.tx.send(FsCmd::FreeSpace {
                        path: self.remote.path.clone(),
                    });
                }
            }
        }

        fn navigate(&mut self, side: Side, path: String) {
            match side {
                Side::Local => {
                    self.local.path = path;
                    self.reload(Side::Local);
                }
                Side::Remote => {
                    self.remote.path = path;
                    self.remote_restore = false;
                    self.reload(Side::Remote);
                }
            }
            self.save_prefs();
        }

        /// Сохранить текущие пути панелей (ui.json рядом с exe).
        fn save_prefs(&self) {
            let mut prefs = load_ui_prefs();
            prefs.files_local = self.local.path.clone();
            prefs
                .files_remote
                .insert(self.addr.clone(), self.remote.path.clone());
            save_ui_prefs(&prefs);
        }

        fn pane(&mut self, side: Side) -> &mut PaneState {
            match side {
                Side::Local => &mut self.local,
                Side::Remote => &mut self.remote,
            }
        }

        /// Первый (по порядку) выделенный объект панели.
        fn first_selected(&self, side: Side) -> Option<(String, bool, u64)> {
            let p = match side {
                Side::Local => &self.local,
                Side::Remote => &self.remote,
            };
            p.sel
                .iter()
                .next()
                .and_then(|&i| p.entries.get(i))
                .map(|e| (e.name.clone(), e.is_dir, e.size))
        }

        /// Все выделенные объекты панели: (имя, папка?, размер).
        fn sel_items(&self, side: Side) -> Vec<(String, bool, u64)> {
            let p = match side {
                Side::Local => &self.local,
                Side::Remote => &self.remote,
            };
            p.sel
                .iter()
                .filter_map(|&i| p.entries.get(i))
                .map(|e| (e.name.clone(), e.is_dir, e.size))
                .collect()
        }

        /// «3 объекта · 12,4 МБ» для нижней строки панели.
        fn sel_info(p: &PaneState) -> String {
            if p.sel.is_empty() {
                return String::new();
            }
            let mut bytes = 0u64;
            for &i in &p.sel {
                if let Some(e) = p.entries.get(i)
                    && !e.is_dir
                {
                    bytes += e.size;
                }
            }
            let n = p.sel.len();
            if bytes > 0 {
                format!(
                    "{n} {} · {}",
                    obj_word(n),
                    rat_session::files::human_size(bytes)
                )
            } else {
                format!("{n} {}", obj_word(n))
            }
        }

        fn tick(&mut self, ctx: &egui::Context) {
            loop {
                match self.rx.try_recv() {
                    Ok(ev) => self.on_event(ev),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        if self.failed.is_none() && self.connected {
                            self.failed = Some("соединение закрыто".to_string());
                        }
                        break;
                    }
                }
            }
            // Горячие клавиши (когда нет модальных окон).
            if self.prompt.is_none() && self.confirm.is_none() && self.conflict.is_none() {
                let (ctrl_a, esc, del) = ctx.input(|i| {
                    (
                        i.modifiers.command && i.key_pressed(egui::Key::A),
                        i.key_pressed(egui::Key::Escape),
                        i.key_pressed(egui::Key::Delete),
                    )
                });
                if ctrl_a {
                    let p = self.pane(self.side);
                    p.sel = (0..p.entries.len()).collect();
                    p.anchor = None;
                }
                if esc {
                    let p = self.pane(self.side);
                    p.sel.clear();
                    p.anchor = None;
                }
                if del {
                    let side = self.side;
                    self.delete_selected(side);
                }
            }
            // Перетаскивание файлов из Проводника прямо на окно.
            for f in ctx.input(|i| i.raw.dropped_files.clone()) {
                let p = f.path().to_path_buf();
                let full = p.to_string_lossy().into_owned();
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| full.clone());
                let is_dir = p.is_dir();
                self.upload_path(full, name, is_dir);
            }
            let now = Instant::now();
            for j in &mut self.jobs {
                if matches!(j.status, JobStatusUi::Active) {
                    let dt = now.duration_since(j.last_t).as_secs_f32();
                    if dt >= 0.5 {
                        j.speed = ((j.done.saturating_sub(j.last_done)) as f32 / dt).max(0.0);
                        j.last_done = j.done;
                        j.last_t = now;
                    }
                }
            }
            if self.failed.is_none() {
                ctx.request_repaint_after(Duration::from_millis(80));
            }
        }

        fn on_event(&mut self, ev: FsEvt) {
            match ev {
                FsEvt::Connected => {
                    self.connected = true;
                    let _ = self.tx.send(FsCmd::Drives);
                    self.reload(Side::Remote);
                }
                FsEvt::ConnectFailed { err } => {
                    self.failed = Some(err);
                }
                FsEvt::List { path, result } => {
                    if path == self.remote.path {
                        self.remote.loading = false;
                        match result {
                            Ok(e) => {
                                self.remote.entries = e;
                                self.remote.sel.clear();
                                self.remote.anchor = None;
                                self.remote_restore = false;
                            }
                            Err(e) => {
                                // Сохранённый путь не открылся — на первый диск.
                                if self.remote_restore
                                    && let Some(d) = self.remote_drives.first().cloned()
                                {
                                    self.navigate(Side::Remote, d);
                                    return;
                                }
                                self.remote.err = Some(e);
                            }
                        }
                    }
                }
                FsEvt::Drives { drives } => {
                    self.remote_drives = drives;
                    // Путь не восстановился сразу — если был отказ, идём на первый диск.
                    if self.remote_restore
                        && self.remote.err.is_some()
                        && let Some(d) = self.remote_drives.first().cloned()
                    {
                        self.navigate(Side::Remote, d);
                    }
                }
                FsEvt::FreeSpace { path, free } => {
                    if path == self.remote.path {
                        self.remote.free = free;
                    }
                }
                FsEvt::Op {
                    action,
                    path,
                    result,
                } => match result {
                    Ok(()) => {
                        self.note(format!("{action}: {path}"));
                        self.reload(Side::Remote);
                    }
                    Err(e) => self.note(format!("{action}: {e}")),
                },
                FsEvt::JobStarted { id } => {
                    if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                        j.status = JobStatusUi::Active;
                        j.last_t = Instant::now();
                    }
                }
                FsEvt::JobNote { id, text, bytes } => {
                    if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                        j.note = text;
                        if bytes > 0 {
                            j.total = bytes;
                        }
                    }
                }
                FsEvt::Progress { id, done, total } => {
                    if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                        j.done = done;
                        j.total = total;
                    }
                }
                FsEvt::JobDone { id, result } => {
                    if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                        j.status = match &result {
                            Ok(()) => {
                                j.done = j.total;
                                JobStatusUi::Done
                            }
                            Err(e) if e == "пропущено" => JobStatusUi::Skipped,
                            Err(e) => JobStatusUi::Failed(e.clone()),
                        };
                    }
                    if result.is_ok() {
                        self.reload(Side::Remote);
                        self.reload(Side::Local);
                    }
                }
                FsEvt::Conflict { id, path } => {
                    let up = self
                        .jobs
                        .iter()
                        .find(|j| j.id == id)
                        .map(|j| j.up)
                        .unwrap_or(false);
                    if let Some(choice) = self.conflict_all {
                        let action = if choice {
                            FsAction::Overwrite
                        } else {
                            FsAction::Skip
                        };
                        let _ = self.tx.send(FsCmd::Resolve { id, action });
                    } else {
                        let name = path.rsplit('\\').next().unwrap_or(&path).to_string();
                        self.conflict = Some(ConflictState {
                            id,
                            path,
                            up,
                            name: name.clone(),
                            choice: 0,
                            apply_all: false,
                            new_name: name,
                        });
                    }
                }
                FsEvt::Disconnected { err } => {
                    self.failed = Some(err);
                }
            }
        }

        fn local_op(
            &mut self,
            action: &'static str,
            f: impl FnOnce() -> std::io::Result<()>,
            path: &str,
        ) {
            match f() {
                Ok(()) => {
                    self.note(format!("{action}: {path}"));
                    self.reload(Side::Local);
                }
                Err(e) => self.note(format!("{action}: ошибка: {e}")),
            }
        }

        fn add_job(&mut self, name: String, is_dir: bool, size: u64, up: bool) -> u64 {
            let id = self.next_job;
            self.next_job += 1;
            self.jobs.push(UiJob {
                id,
                up,
                name,
                total: size,
                done: 0,
                status: JobStatusUi::Queued,
                speed: 0.0,
                last_done: 0,
                last_t: Instant::now(),
                note: String::new(),
                is_dir,
            });
            id
        }

        /// «Скачать» (кнопка на правой панели): выделенное на удалённой
        /// машине → в текущую папку локальной панели.
        fn act_download_selected(&mut self) {
            let items = self.sel_items(Side::Remote);
            if items.is_empty() {
                self.note("выберите файлы в правой панели".to_string());
                return;
            }
            for (name, is_dir, size) in items {
                self.begin_download(name, is_dir, size);
            }
        }

        /// «Загрузить» (кнопка на левой панели): выделенное локально →
        /// в текущую папку удалённой машины.
        fn act_upload_selected(&mut self) {
            let items = self.sel_items(Side::Local);
            if items.is_empty() {
                self.note("выберите файлы в левой панели".to_string());
                return;
            }
            for (name, is_dir, size) in items {
                self.begin_upload(name, is_dir, size);
            }
        }

        /// Удаление выделенного (одним подтверждением на всех).
        fn delete_selected(&mut self, side: Side) {
            let items = self.sel_items(side);
            if items.is_empty() {
                self.note("выберите файлы или папки".to_string());
                return;
            }
            let base = match side {
                Side::Local => self.local.path.clone(),
                Side::Remote => self.remote.path.clone(),
            };
            let paths = items
                .into_iter()
                .map(|(name, is_dir, _)| {
                    let full = join_path(&base, &name);
                    (full, name, is_dir)
                })
                .collect();
            self.confirm = Some(ConfirmState { side, items: paths });
        }

        fn begin_download(&mut self, name: String, is_dir: bool, size: u64) {
            if !self.connected {
                self.note("дождитесь соединения".to_string());
                return;
            }
            let remote = join_path(&self.remote.path, &name);
            let local = join_path(&self.local.path, &name);
            let id = self.add_job(name, is_dir, size, false);
            if is_dir {
                let _ = self.tx.send(FsCmd::DownloadFolder { id, remote, local });
            } else {
                let _ = self.tx.send(FsCmd::Download { id, remote, local });
            }
        }

        fn begin_upload(&mut self, name: String, is_dir: bool, size: u64) {
            if !self.connected {
                self.note("дождитесь соединения".to_string());
                return;
            }
            let local = join_path(&self.local.path, &name);
            let remote = join_path(&self.remote.path, &name);
            let id = self.add_job(name, is_dir, size, true);
            if is_dir {
                let _ = self.tx.send(FsCmd::UploadFolder { id, local, remote });
            } else {
                let _ = self.tx.send(FsCmd::Upload { id, local, remote });
            }
        }

        /// Загрузка произвольного локального пути (перетаскивание из Проводника).
        fn upload_path(&mut self, full: String, name: String, is_dir: bool) {
            if !self.connected {
                self.note("дождитесь соединения".to_string());
                return;
            }
            let size = if is_dir {
                0
            } else {
                std::fs::metadata(&full).map(|m| m.len()).unwrap_or(0)
            };
            let remote = join_path(&self.remote.path, &name);
            let id = self.add_job(name, is_dir, size, true);
            if is_dir {
                let _ = self.tx.send(FsCmd::UploadFolder {
                    id,
                    local: full,
                    remote,
                });
            } else {
                let _ = self.tx.send(FsCmd::Upload {
                    id,
                    local: full,
                    remote,
                });
            }
        }

        fn ui(&mut self, ui: &mut egui::Ui) {
            egui::Panel::top(egui::Id::new("files_top")).show(ui, |ui| self.top_ui(ui));
            egui::Panel::bottom(egui::Id::new("files_queue"))
                .exact_size(118.0)
                .show(ui, |ui| self.queue_ui(ui));
            egui::CentralPanel::default().show(ui, |ui| self.panes_ui(ui));
        }

        fn top_ui(&mut self, ui: &mut egui::Ui) {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("← Назад").clicked() {
                    self.want_close = true;
                }
                ui.add_space(6.0);
                let (dot, text) = if let Some(e) = &self.failed {
                    (egui::Color32::from_rgb(203, 60, 60), format!("ошибка: {e}"))
                } else if self.connected {
                    (
                        egui::Color32::from_rgb(34, 160, 107),
                        format!("Файловый режим (без экрана) — {}", self.name),
                    )
                } else {
                    (
                        egui::Color32::from_rgb(195, 200, 207),
                        format!("подключение… {}", self.addr),
                    )
                };
                ui.label(egui::RichText::new("●").color(dot).size(13.0));
                ui.label(text);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Отключиться").clicked() {
                        self.want_close = true;
                    }
                    let speed: f32 = self
                        .jobs
                        .iter()
                        .filter(|j| matches!(j.status, JobStatusUi::Active))
                        .map(|j| j.speed)
                        .sum();
                    if speed > 1.0 {
                        ui.label(
                            egui::RichText::new(format!(
                                "канал: {}/с",
                                rat_session::files::human_size(speed as u64)
                            ))
                            .color(DIM2)
                            .size(12.0),
                        );
                    }
                    if let Some(st) = &self.status
                        && self.status_t.elapsed().as_secs() < 7
                    {
                        ui.label(egui::RichText::new(st.clone()).color(DIM2).size(12.5));
                    }
                });
            });
            ui.add_space(3.0);
        }

        fn queue_ui(&mut self, ui: &mut egui::Ui) {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Передачи").strong());
                ui.label(
                    egui::RichText::new(format!("({})", self.jobs.len()))
                        .color(DIM2)
                        .size(12.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Очистить завершённые").clicked() {
                        self.jobs.retain(|j| {
                            matches!(j.status, JobStatusUi::Queued | JobStatusUi::Active)
                        });
                    }
                });
            });
            let mut cancel: Option<u64> = None;
            egui::ScrollArea::vertical()
                .max_height(76.0)
                .show(ui, |ui| {
                    if self.jobs.is_empty() {
                        ui.label(egui::RichText::new("очередь пуста").color(DIM2).size(12.0));
                    }
                    for j in &self.jobs {
                        ui.horizontal(|ui| {
                            ui.label(if j.up { "→" } else { "←" });
                            let nm = if j.is_dir {
                                format!("{} (папка)", j.name)
                            } else {
                                j.name.clone()
                            };
                            ui.label(nm);
                            if !j.note.is_empty() && matches!(j.status, JobStatusUi::Active) {
                                ui.label(
                                    egui::RichText::new(j.note.clone()).size(11.5).color(DIM2),
                                );
                            }
                            let frac = if j.total > 0 {
                                (j.done as f32 / j.total as f32).clamp(0.0, 1.0)
                            } else if matches!(j.status, JobStatusUi::Done) {
                                1.0
                            } else {
                                0.0
                            };
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(200.0, 8.0), egui::Sense::hover());
                            ui.painter().rect_filled(
                                rect,
                                4.0,
                                egui::Color32::from_rgb(232, 235, 239),
                            );
                            if frac > 0.0 {
                                let mut fr = rect;
                                fr.set_width(rect.width() * frac);
                                ui.painter().rect_filled(
                                    fr,
                                    4.0,
                                    egui::Color32::from_rgb(75, 141, 240),
                                );
                            }
                            let st = match &j.status {
                                JobStatusUi::Queued => "в очереди".to_string(),
                                JobStatusUi::Active => format!(
                                    "{} / {} · {}/с",
                                    rat_session::files::human_size(j.done),
                                    rat_session::files::human_size(j.total),
                                    rat_session::files::human_size(j.speed as u64)
                                ),
                                JobStatusUi::Done => {
                                    format!("готово · {}", rat_session::files::human_size(j.total))
                                }
                                JobStatusUi::Skipped => "пропущено".to_string(),
                                JobStatusUi::Failed(e) => format!("ошибка: {e}"),
                            };
                            ui.label(egui::RichText::new(st).size(12.0).color(DIM2));
                            if matches!(j.status, JobStatusUi::Active | JobStatusUi::Queued)
                                && ui.button("✕").clicked()
                            {
                                cancel = Some(j.id);
                            }
                        });
                    }
                });
            if let Some(id) = cancel {
                let _ = self.tx.send(FsCmd::Cancel { id });
            }
        }

        fn panes_ui(&mut self, ui: &mut egui::Ui) {
            ui.columns(2, |cols| {
                self.pane_ui(&mut cols[0], Side::Local);
                self.pane_ui(&mut cols[1], Side::Remote);
            });
            if ui.input(|i| i.pointer.any_released()) {
                // Отпустили не над панелью-приёмником — перенос отменяется.
                self.drag_src = None;
            }
        }

        fn pane_ui(&mut self, ui: &mut egui::Ui, side: Side) {
            let resp = egui::Frame::NONE
                .fill(egui::Color32::from_rgb(252, 252, 253))
                .stroke(egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgb(226, 229, 232),
                ))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    let full = ui.available_size();
                    ui.set_min_size(full);
                    self.pane_body(ui, side);
                });
            let pane_rect = resp.response.rect;
            // Клик по панели делает её активной (Ctrl+A, Delete и т.п.).
            if ui.rect_contains_pointer(pane_rect) && ui.input(|i| i.pointer.any_click()) {
                self.side = side;
            }
            // Перетаскивание между панелями: подсветка приёмника и приём.
            if let Some(d) = self.drag_src.clone()
                && d.side != side
                && ui.rect_contains_pointer(pane_rect)
            {
                ui.painter().rect_filled(
                    pane_rect,
                    8.0,
                    egui::Color32::from_rgba_unmultiplied(75, 141, 240, 18),
                );
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                if ui.input(|i| i.pointer.any_released()) {
                    self.drop_drag(side);
                }
            }
        }

        /// Принять перетаскивание в панель `target`.
        fn drop_drag(&mut self, target: Side) {
            let Some(d) = self.drag_src.take() else {
                return;
            };
            match (d.side, target) {
                (Side::Local, Side::Remote) => {
                    for (name, is_dir, size) in d.items {
                        self.begin_upload(name, is_dir, size);
                    }
                }
                (Side::Remote, Side::Local) => {
                    for (name, is_dir, size) in d.items {
                        self.begin_download(name, is_dir, size);
                    }
                }
                _ => {}
            }
        }

        fn pane_body(&mut self, ui: &mut egui::Ui, side: Side) {
            let is_active = self.side == side;
            // Заголовок панели: имя + свободное место.
            ui.horizontal(|ui| {
                let title = match side {
                    Side::Local => "Этот компьютер".to_string(),
                    Side::Remote => self.name.clone(),
                };
                let t = egui::RichText::new(title).strong().size(14.5);
                if is_active {
                    ui.label(t.color(egui::Color32::from_rgb(29, 79, 159)));
                } else {
                    ui.label(t);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let free = match side {
                        Side::Local => self.local.free,
                        Side::Remote => self.remote.free,
                    };
                    if let Some((free, _total)) = free {
                        ui.label(
                            egui::RichText::new(format!(
                                "свободно {}",
                                rat_session::files::human_size(free)
                            ))
                            .color(DIM2)
                            .size(12.0),
                        );
                    }
                });
            });
            ui.add_space(2.0);
            // Навигация: свои кнопки в каждой панели + кликабельный путь.
            ui.horizontal(|ui| {
                let path = self.pane(side).path.clone();
                if ui.small_button("⬆ Вверх").clicked()
                    && let Some(p) = up_dir(&path)
                {
                    self.navigate(side, p);
                }
                if ui.small_button("⟳ Обновить").clicked() {
                    self.reload(side);
                }
                ui.separator();
                let crumbs = crumbs(&path);
                let mut jump: Option<String> = None;
                let n = crumbs.len();
                for (k, (label, full)) in crumbs.into_iter().enumerate() {
                    if k > 0 {
                        ui.label(egui::RichText::new("›").color(DIM2).size(11.0));
                    }
                    let cur = k + 1 == n;
                    let text = if cur {
                        egui::RichText::new(&label)
                            .strong()
                            .color(egui::Color32::from_rgb(35, 38, 42))
                    } else {
                        egui::RichText::new(&label).color(egui::Color32::from_rgb(52, 83, 126))
                    };
                    let r = ui.add(egui::Button::new(text).frame(false));
                    if !cur && r.clicked() {
                        jump = Some(full.clone());
                    }
                }
                if let Some(p) = jump {
                    self.navigate(side, p);
                }
            });
            ui.add_space(2.0);
            // Диски — горизонтальный ряд кнопок.
            ui.horizontal(|ui| {
                let drives = match side {
                    Side::Local => rat_session::files::local_drives(),
                    Side::Remote => self.remote_drives.clone(),
                };
                ui.label(egui::RichText::new("Диски:").color(DIM2).size(12.0));
                let cur = self.pane(side).path.clone();
                let mut jump: Option<String> = None;
                for d in &drives {
                    let on = cur.starts_with(d.as_str());
                    let label = d.trim_end_matches('\\');
                    if ui
                        .selectable_label(on, egui::RichText::new(label).size(12.5))
                        .clicked()
                    {
                        jump = Some(d.clone());
                    }
                }
                if drives.is_empty() {
                    ui.label(egui::RichText::new("нет данных").color(DIM2).size(12.0));
                }
                if let Some(p) = jump {
                    self.navigate(side, p);
                }
            });
            ui.add_space(2.0);
            // Список файлов: прокрутка независима, колесо — над своей панелью.
            let scroll_h = (ui.available_height() - 34.0).max(90.0);
            let mut dbl: Option<(bool, String, u64, String)> = None;
            let mut drag_new: Option<DragPayload> = None;
            let mut clicked = false;
            {
                let pane = self.pane(side);
                let pane_len = pane.entries.len();
                egui::ScrollArea::vertical()
                    .id_salt(("files_pane", side == Side::Local))
                    .auto_shrink([false, false])
                    .max_height(scroll_h)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if pane.loading {
                            ui.label(egui::RichText::new("загрузка…").color(DIM2).size(12.0));
                        }
                        if let Some(e) = &pane.err {
                            ui.label(
                                egui::RichText::new(format!("ошибка: {e}"))
                                    .color(egui::Color32::from_rgb(203, 60, 60))
                                    .size(12.0),
                            );
                        }
                        if pane.entries.is_empty() && !pane.loading {
                            ui.label(egui::RichText::new("пусто").color(DIM2).size(12.0));
                        }
                        for (i, e) in pane.entries.iter().enumerate() {
                            let selected = pane.sel.contains(&i);
                            let resp = files_row(ui, e, selected);
                            if resp.clicked() {
                                let (ctrl, shift) =
                                    ui.input(|i2| (i2.modifiers.command, i2.modifiers.shift));
                                sel_click(
                                    &mut pane.sel,
                                    &mut pane.anchor,
                                    i,
                                    pane_len,
                                    ctrl,
                                    shift,
                                );
                                clicked = true;
                            }
                            if resp.double_clicked() {
                                pane.sel.clear();
                                pane.sel.insert(i);
                                pane.anchor = Some(i);
                                clicked = true;
                                dbl = Some((
                                    e.is_dir,
                                    e.name.clone(),
                                    e.size,
                                    join_path(&pane.path, &e.name),
                                ));
                            }
                            if resp.drag_started() {
                                let items: Vec<(String, bool, u64)> = if pane.sel.contains(&i) {
                                    pane.sel
                                        .iter()
                                        .filter_map(|&k| pane.entries.get(k))
                                        .map(|e2| (e2.name.clone(), e2.is_dir, e2.size))
                                        .collect()
                                } else {
                                    pane.sel.clear();
                                    pane.sel.insert(i);
                                    pane.anchor = Some(i);
                                    vec![(e.name.clone(), e.is_dir, e.size)]
                                };
                                drag_new = Some(DragPayload { side, items });
                            }
                        }
                        // Клик по пустому месту — снять выделение.
                        let gap = ui.available_size();
                        if gap.y > 8.0 {
                            let resp = ui.allocate_response(gap, egui::Sense::click());
                            if resp.clicked() && !ui.input(|i2| i2.modifiers.command) {
                                pane.sel.clear();
                                pane.anchor = None;
                                clicked = true;
                            }
                        }
                    });
            }
            if clicked {
                self.side = side;
            }
            if let Some(d) = drag_new {
                self.drag_src = Some(d);
            }
            if let Some((is_dir, name, size, full)) = dbl {
                if is_dir {
                    self.navigate(side, full);
                } else if side == Side::Remote {
                    self.begin_download(name, false, size);
                } else {
                    self.begin_upload(name, false, size);
                }
            }
            // Нижняя строка панели: свои кнопки действий.
            ui.horizontal(|ui| {
                match side {
                    Side::Local => {
                        let b = egui::Button::new(
                            egui::RichText::new("⬆ Загрузить").color(egui::Color32::WHITE),
                        )
                        .fill(egui::Color32::from_rgb(46, 111, 208));
                        if ui.add(b).clicked() {
                            self.act_upload_selected();
                        }
                    }
                    Side::Remote => {
                        let b = egui::Button::new(
                            egui::RichText::new("⬇ Скачать").color(egui::Color32::WHITE),
                        )
                        .fill(egui::Color32::from_rgb(46, 111, 208));
                        if ui.add(b).clicked() {
                            self.act_download_selected();
                        }
                    }
                }
                if ui.button("Удалить").clicked() {
                    self.delete_selected(side);
                }
                let sel_n = self.pane(side).sel.len();
                let rename = ui.add_enabled(sel_n == 1, egui::Button::new("Переименовать"));
                if rename.clicked()
                    && let Some((name, _, _)) = self.first_selected(side)
                {
                    self.prompt = Some(PromptState {
                        kind: PromptKind::Rename,
                        side,
                        text: name.clone(),
                        target: name,
                    });
                }
                if ui.button("Новая папка").clicked() {
                    self.prompt = Some(PromptState {
                        kind: PromptKind::NewFolder,
                        side,
                        text: "Новая папка".to_string(),
                        target: String::new(),
                    });
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let info = match side {
                        Side::Local => Self::sel_info(&self.local),
                        Side::Remote => Self::sel_info(&self.remote),
                    };
                    if !info.is_empty() {
                        ui.label(egui::RichText::new(info).color(DIM2).size(11.5));
                    }
                });
            });
        }

        fn dialogs(&mut self, ctx: &egui::Context) {
            // Конфликт имён.
            if self.conflict.is_some() {
                let mut answer: Option<FsAction> = None;
                let mut cancel = false;
                {
                    let c = self.conflict.as_mut().unwrap();
                    egui::Window::new("Файл уже существует")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(ctx, |ui| {
                            let where_ = if c.up {
                                "на удалённой машине"
                            } else {
                                "в папке назначения"
                            };
                            ui.label(format!("«{}» уже есть {where_}.", c.name));
                            ui.label(
                                egui::RichText::new(&c.path)
                                    .monospace()
                                    .size(11.0)
                                    .color(DIM2),
                            );
                            ui.add_space(4.0);
                            ui.radio_value(&mut c.choice, 0u8, "Заменить");
                            ui.radio_value(&mut c.choice, 1u8, "Пропустить");
                            ui.radio_value(&mut c.choice, 2u8, "Сохранить как…");
                            if c.choice == 2 {
                                ui.text_edit_singleline(&mut c.new_name);
                            }
                            ui.checkbox(&mut c.apply_all, "Применить ко всем файлам");
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if ui.button("Отмена").clicked() {
                                    cancel = true;
                                }
                                let can_ok = c.choice != 2 || !c.new_name.trim().is_empty();
                                if ui.button("Далее").clicked() && can_ok {
                                    answer = Some(match c.choice {
                                        0 => FsAction::Overwrite,
                                        1 => FsAction::Skip,
                                        _ => FsAction::Rename(c.new_name.trim().to_string()),
                                    });
                                }
                            });
                        });
                }
                if cancel || answer.is_some() {
                    let c = self.conflict.take().unwrap();
                    let action = answer.unwrap_or(FsAction::Skip);
                    if c.apply_all {
                        self.conflict_all = match action {
                            FsAction::Overwrite => Some(true),
                            FsAction::Skip => Some(false),
                            FsAction::Rename(_) => None,
                        };
                    }
                    let _ = self.tx.send(FsCmd::Resolve { id: c.id, action });
                }
            }
            // Новая папка / переименование.
            if self.prompt.is_some() {
                let (kind, side) = {
                    let p = self.prompt.as_ref().unwrap();
                    (p.kind, p.side)
                };
                let mut ok = false;
                let mut cancel = false;
                {
                    let p = self.prompt.as_mut().unwrap();
                    let title = match kind {
                        PromptKind::NewFolder => "Новая папка",
                        PromptKind::Rename => "Переименовать",
                    };
                    egui::Window::new(title)
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(ctx, |ui| {
                            ui.label("Имя:");
                            ui.text_edit_singleline(&mut p.text);
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if ui.button("Отмена").clicked() {
                                    cancel = true;
                                }
                                if ui.button("ОК").clicked() {
                                    ok = true;
                                }
                            });
                        });
                }
                if ok || cancel {
                    let p = self.prompt.take().unwrap();
                    let name = p.text.trim().to_string();
                    if ok && !name.is_empty() {
                        self.apply_prompt(kind, side, &name, &p.target);
                    }
                }
            }
            // Подтверждение удаления (может быть сразу несколько объектов).
            if self.confirm.is_some() {
                let mut ok = false;
                let mut cancel = false;
                {
                    let c = self.confirm.as_ref().unwrap();
                    egui::Window::new("Удаление")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(ctx, |ui| {
                            let n = c.items.len();
                            if n == 1 {
                                let (_, name, is_dir) = &c.items[0];
                                if *is_dir {
                                    ui.label(format!("Удалить папку «{name}» со всем содержимым?"));
                                } else {
                                    ui.label(format!("Удалить файл «{name}»? (без корзины)"));
                                }
                            } else {
                                ui.label(format!("Удалить {} {} (без корзины)?", n, obj_word(n)));
                                let names: Vec<&str> =
                                    c.items.iter().take(5).map(|(_, n, _)| n.as_str()).collect();
                                let mut line = names.join(", ");
                                if n > 5 {
                                    line.push_str(", …");
                                }
                                ui.label(egui::RichText::new(line).color(DIM2).size(12.0));
                            }
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if ui.button("Отмена").clicked() {
                                    cancel = true;
                                }
                                if ui.button("Удалить").clicked() {
                                    ok = true;
                                }
                            });
                        });
                }
                if ok || cancel {
                    let c = self.confirm.take().unwrap();
                    if ok {
                        match c.side {
                            Side::Local => {
                                let mut failed: Vec<String> = Vec::new();
                                for (path, _, is_dir) in &c.items {
                                    let r = if *is_dir {
                                        std::fs::remove_dir_all(path)
                                    } else {
                                        std::fs::remove_file(path)
                                    };
                                    if let Err(e) = r {
                                        failed.push(format!("{path}: {e}"));
                                    }
                                }
                                let n = c.items.len();
                                if failed.is_empty() {
                                    self.note(format!("удалено: {n} {}", obj_word(n)));
                                } else {
                                    self.note(format!("удаление: {}", failed.join("; ")));
                                }
                                self.reload(Side::Local);
                            }
                            Side::Remote => {
                                for (path, _, _) in &c.items {
                                    let _ = self.tx.send(FsCmd::Delete { path: path.clone() });
                                }
                            }
                        }
                    }
                }
            }
        }

        fn apply_prompt(&mut self, kind: PromptKind, side: Side, name: &str, target: &str) {
            let base = match side {
                Side::Local => self.local.path.clone(),
                Side::Remote => self.remote.path.clone(),
            };
            match kind {
                PromptKind::NewFolder => {
                    let path = join_path(&base, name);
                    match side {
                        Side::Local => {
                            let p2 = path.clone();
                            self.local_op("папка создана", move || std::fs::create_dir(&p2), &path);
                        }
                        Side::Remote => {
                            let _ = self.tx.send(FsCmd::Mkdir { path });
                        }
                    }
                }
                PromptKind::Rename => {
                    let old = join_path(&base, target);
                    let new = join_path(&base, name);
                    match side {
                        Side::Local => {
                            let a = old.clone();
                            let b = new.clone();
                            self.local_op("переименовано", move || std::fs::rename(&a, &b), &new);
                        }
                        Side::Remote => {
                            let _ = self.tx.send(FsCmd::Rename { old, new });
                        }
                    }
                }
            }
        }
    }

    fn join_path(dir: &str, name: &str) -> String {
        if dir.ends_with('\\') || dir.ends_with('/') {
            format!("{dir}{name}")
        } else {
            format!("{dir}\\{name}")
        }
    }

    fn up_dir(path: &str) -> Option<String> {
        let p = std::path::Path::new(path);
        p.parent().map(|d| d.to_string_lossy().into_owned())
    }

    fn list_dir(path: &str) -> Result<Vec<FsDirEntry>, String> {
        let rd = std::fs::read_dir(path).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Ok(md) = e.metadata() {
                let is_dir = md.is_dir();
                let mtime = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                out.push(FsDirEntry {
                    name,
                    is_dir,
                    size: if is_dir { 0 } else { md.len() },
                    mtime,
                });
            }
        }
        out.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(out)
    }

    fn local_home_dir() -> String {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string());
        let docs = format!("{home}\\Documents");
        if std::path::Path::new(&docs).is_dir() {
            docs
        } else {
            home
        }
    }

    fn files_row(ui: &mut egui::Ui, e: &FsDirEntry, selected: bool) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 26.0),
            egui::Sense::click_and_drag(),
        );
        if selected {
            ui.painter()
                .rect_filled(rect, 6.0, egui::Color32::from_rgb(207, 228, 250));
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect, 6.0, egui::Color32::from_rgb(240, 244, 248));
        }
        let ic = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 8.0, rect.center().y - 7.5),
            egui::vec2(15.0, 15.0),
        );
        if e.is_dir {
            ui.painter()
                .rect_filled(ic, 3.0, egui::Color32::from_rgb(243, 178, 76));
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(ic.left(), ic.top() - 3.0),
                    egui::vec2(7.0, 4.5),
                ),
                1.5,
                egui::Color32::from_rgb(243, 178, 76),
            );
        } else {
            ui.painter().rect_filled(ic, 3.5, icon_color(&e.name));
        }
        let clip = ui.painter().with_clip_rect(rect);
        clip.text(
            egui::pos2(rect.left() + 30.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &e.name,
            egui::FontId::proportional(13.0),
            egui::Color32::from_rgb(31, 35, 40),
        );
        let sz = if e.is_dir {
            "папка".to_string()
        } else {
            rat_session::files::human_size(e.size)
        };
        clip.text(
            egui::pos2(rect.right() - 6.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            sz,
            egui::FontId::proportional(11.5),
            DIM2,
        );
        resp
    }

    /// Цвет иконки файла по типу (как на согласованном макете).
    fn icon_color(name: &str) -> egui::Color32 {
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        let (r, g, b) = match ext.as_str() {
            "xlsx" | "xls" | "xlsm" | "ods" | "csv" => (86, 166, 100),
            "doc" | "docx" | "odt" | "rtf" => (77, 137, 200),
            "ppt" | "pptx" | "odp" => (219, 100, 86),
            "pdf" => (216, 85, 85),
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "ico" | "svg" | "heic" => {
                (90, 150, 220)
            }
            "zip" | "7z" | "rar" | "tar" | "gz" | "xz" => (155, 122, 210),
            "exe" | "msi" | "dll" | "sys" | "bat" | "cmd" | "ps1" => (238, 158, 68),
            _ => (155, 164, 175),
        };
        egui::Color32::from_rgb(r, g, b)
    }

    /// «1 объект / 2 объекта / 5 объектов».
    fn obj_word(n: usize) -> &'static str {
        let d100 = n % 100;
        let d10 = n % 10;
        if (11..=14).contains(&d100) {
            "объектов"
        } else {
            match d10 {
                1 => "объект",
                2..=4 => "объекта",
                _ => "объектов",
            }
        }
    }

    /// Клик по строке: обычный — один объект; Ctrl — добавить/убрать;
    /// Shift — диапазон от «якоря»; Shift+Ctrl — добавить диапазон.
    fn sel_click(
        sel: &mut BTreeSet<usize>,
        anchor: &mut Option<usize>,
        idx: usize,
        count: usize,
        ctrl: bool,
        shift: bool,
    ) {
        if shift
            && let Some(a) = *anchor
            && a < count
            && idx < count
        {
            let (lo, hi) = if a <= idx { (a, idx) } else { (idx, a) };
            if !ctrl {
                sel.clear();
            }
            for k in lo..=hi {
                sel.insert(k);
            }
            return;
        }
        if ctrl {
            if !sel.remove(&idx) {
                sel.insert(idx);
            }
            *anchor = Some(idx);
            return;
        }
        sel.clear();
        sel.insert(idx);
        *anchor = Some(idx);
    }

    /// Путь → «хлебные крошки»: (подпись, полный путь до этого уровня).
    fn crumbs(path: &str) -> Vec<(String, String)> {
        let norm = path.replace('/', "\\");
        let mut out: Vec<(String, String)> = Vec::new();
        if norm.is_empty() {
            return out;
        }
        if let Some(rest) = norm.strip_prefix("\\\\") {
            let mut it = rest.splitn(3, '\\');
            let srv = it.next().unwrap_or("").to_string();
            let share = it.next().unwrap_or("").to_string();
            if !srv.is_empty() {
                out.push((srv.clone(), format!("\\\\{srv}\\")));
            }
            if !share.is_empty() {
                let mut acc = format!("\\\\{srv}\\{share}");
                out.push((share, acc.clone()));
                for comp in it
                    .next()
                    .unwrap_or("")
                    .split('\\')
                    .filter(|s| !s.is_empty())
                {
                    acc = format!("{acc}\\{comp}");
                    out.push((comp.to_string(), acc.clone()));
                }
            }
            return out;
        }
        let mut acc = String::new();
        let mut comps = norm.split('\\').filter(|s| !s.is_empty()).peekable();
        if let Some(first) = comps.peek()
            && first.ends_with(':')
        {
            let d = first.to_string();
            comps.next();
            acc = format!("{d}\\");
            out.push((d, acc.clone()));
        }
        for comp in comps {
            if acc.is_empty() {
                acc = comp.to_string();
            } else if acc.ends_with('\\') {
                acc = format!("{acc}{comp}");
            } else {
                acc = format!("{acc}\\{comp}");
            }
            out.push((comp.to_string(), acc.clone()));
        }
        out
    }

    /// Запомненные пути панелей (ui.json рядом с exe).
    #[derive(Default, Serialize, Deserialize)]
    struct UiPrefs {
        #[serde(default)]
        files_local: String,
        #[serde(default)]
        files_remote: BTreeMap<String, String>,
    }

    fn ui_prefs_path() -> std::path::PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("ui.json")))
            .unwrap_or_else(|| std::path::PathBuf::from("ui.json"))
    }

    fn load_ui_prefs() -> UiPrefs {
        std::fs::read_to_string(ui_prefs_path())
            .ok()
            .and_then(|s| serde_json::from_str::<UiPrefs>(&s).ok())
            .unwrap_or_default()
    }

    fn save_ui_prefs(p: &UiPrefs) {
        if let Ok(s) = serde_json::to_string_pretty(p) {
            let _ = std::fs::write(ui_prefs_path(), s);
        }
    }

    enum Screen {
        Home,
        Viewer,
        Files,
    }

    struct ViewerSession {
        name: String,
        addr: String,
        rx: Receiver<FrameOut>,
        stats: Arc<Mutex<SessionStats>>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
        closer: TcpCloser,
        base: TimeBase,
        texture: Option<egui::TextureHandle>,
        last: Option<FrameOut>,
        input_q: Arc<Mutex<Vec<InputEvent>>>,
        cursor: Arc<Mutex<CursorState>>,
        info: Arc<Mutex<Option<SessionInfo>>>,
        monitors: Arc<Mutex<Vec<(u16, u16, u16)>>>,
        pending_switch: Arc<Mutex<Option<u16>>>,
        // Панели-оверлеи: автоскрытие и анимация выезда.
        top_open: bool,
        bottom_open: bool,
        top_t: f32,
        bottom_t: f32,
        pinned: bool,
        clip_in: Arc<Mutex<Option<String>>>,
        pending_paste: Arc<Mutex<Option<String>>>,
        last_clip_applied: Option<String>,
        switch_after: Option<f32>,
        switch_done: bool,
        connected_at: Instant,
        last_pointer: Option<(u16, u16)>,
        /// Диагностика/предсказание курсора (Шаг 0/1): последняя позиция из
        /// пакетов агента, время её изменения и время последней отправки ввода.
        last_cursor_pos: Option<(u16, u16)>,
        last_cursor_change_at: Instant,
        last_move_at: Instant,
        prev_mods: egui::Modifiers,
        buttons_down: [bool; 5],
        ui_e2e_ms: Vec<f32>,
        cursor_img: Arc<Mutex<Option<rat_session::viewer::CursorImage>>>,
        cursor_img_ver: Arc<std::sync::atomic::AtomicU64>,
        cursor_tex: Option<egui::TextureHandle>,
        cursor_tex_ver: u64,
    }

    impl ViewerSession {
        fn shutdown(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
            self.closer.close();
            let st = self.stats.lock().unwrap();
            crate::note(&format!(
                "=== SESSION SUMMARY ({}) ===\n{}",
                self.addr,
                st.summary()
            ));
            if !self.ui_e2e_ms.is_empty() {
                crate::note(&format!(
                    "ui present e2e: avg {:.2}ms p95 {:.2}ms",
                    SessionStats::avg(&self.ui_e2e_ms),
                    SessionStats::pct(&self.ui_e2e_ms, 0.95)
                ));
            }
        }
    }

    enum TileAction {
        Connect(usize),
        Files(usize),
        Rename(usize),
        Delete(usize),
        Add,
    }

    pub struct RatApp {
        args: Args,
        machines: Arc<Mutex<Vec<Machine>>>,
        statuses: Arc<Mutex<HashMap<String, bool>>>,
        server: ServerHandle,
        internet: InternetHandle,
        device: Device,
        settings_open: bool,
        settings_mode: ConnectionMode,
        settings_relay_edit: String,
        settings_auto_allow: bool,
        settings_error: Option<String>,
        /// «Служебное»: локальный приём и его порт (применяются по «Сохранить»).
        settings_serve_local: bool,
        settings_serve_port: String,

        incoming_rx: Receiver<Request>,
        incoming_pending: Option<Request>,

        my_ip: String,
        my_name: String,
        screen: Screen,
        viewer: Option<ViewerSession>,
        files: Option<FilesState>,
        title_now: String,
        connect_rx: Option<Receiver<ConnectMsg>>,
        connecting_to: Option<String>,
        toast: Option<(String, Instant)>,
        add_open: bool,
        add_name: String,
        add_addr: String,
        add_err: Option<String>,
        edit_open: Option<usize>,
        edit_name: String,
        autoconnect_done: bool,
        files_ui_done: bool,
        started: Instant,
        finished: bool,
        /// Значок в области уведомлений (создаётся всегда; если оболочка
        /// ещё не готова — попытки повторяются).
        tray: Option<crate::tray::Tray>,
        /// Осталось повторных попыток создать значок.
        tray_retry_left: u32,
        /// Когда повторить попытку создания значка.
        tray_retry_at: Instant,
        /// Помощник службы, переведённый в обычный режим (служба остановлена).
        svc_detached: bool,
        /// Время последней проверки «служба ещё работает?» для помощника.
        svc_probe_checked: Instant,
        /// Когда закрыть окно после передачи управления службе (установка).
        quit_at: Option<Instant>,
        /// Выбран «Выход» — закрытие окна завершает процесс, а не прячет его.
        quitting: bool,
        /// Окно скрыто в трей — перерисовку можно замедлить.
        hidden: bool,
        /// Первый кадр: при `--tray` спрятать окно.
        tray_hide_done: bool,
        /// Окно показали автоматически ради диалога подтверждения —
        /// после ответа его нужно вернуть в трей.
        shown_for_dialog: bool,
        /// Запущено ли с повышенными правами (иначе действует UIPI-ограничение).
        elevated: bool,
        /// Состояние службы «RatNext» (для окна настроек).
        #[cfg(feature = "ffmpeg")]
        svc_state: crate::service::State,
        /// Время последнего опроса состояния службы.
        #[cfg(feature = "ffmpeg")]
        svc_checked: Instant,
        /// Запрос «показать окно» от повторного запуска exe.
        show_request: Option<rat_session::single_instance::ShowListener>,
        /// Запрос вежливого завершения (установщик, скрипты обновления).
        quit_request: Option<rat_session::single_instance::QuitListener>,
        /// Контекст egui для слушателя повторного запуска (будит перерисовку).
        repaint_slot: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>,
    }

    impl RatApp {
        fn new(
            args: Args,
            show_request: Option<rat_session::single_instance::ShowListener>,
            quit_request: Option<rat_session::single_instance::QuitListener>,
            repaint_slot: std::sync::Arc<std::sync::Mutex<Option<egui::Context>>>,
        ) -> Self {
            let machines = Arc::new(Mutex::new(load()));
            let statuses: Arc<Mutex<HashMap<String, bool>>> = Arc::new(Mutex::new(HashMap::new()));
            {
                let machines = machines.clone();
                let statuses = statuses.clone();
                std::thread::spawn(move || {
                    loop {
                        let device = crate::store::load_device();
                        let list: Vec<String> = machines
                            .lock()
                            .unwrap()
                            .iter()
                            .map(|m| m.addr.clone())
                            .collect();
                        for addr in list {
                            let ok = match device.mode {
                                ConnectionMode::Lan => addr
                                    .to_socket_addrs()
                                    .ok()
                                    .and_then(|mut it| it.next())
                                    .map(|sa| {
                                        std::net::TcpStream::connect_timeout(
                                            &sa,
                                            Duration::from_millis(500),
                                        )
                                        .is_ok()
                                    })
                                    .unwrap_or(false),
                                ConnectionMode::Internet => {
                                    rat_session::link::relay_who(device.relay.as_str(), &addr)
                                        .unwrap_or(false)
                                }
                            };
                            statuses.lock().unwrap().insert(addr, ok);
                        }
                        std::thread::sleep(Duration::from_secs(4));
                    }
                });
            }
            let port = args.port;
            let (incoming_tx, incoming_rx) = channel();
            let mut server = ServerHandle::new(incoming_tx.clone());
            let mut toast = None;
            if let Err(e) = server.start(port) {
                crate::note(&format!("[ui] сервер не запустился: {e}"));
                toast = Some((format!("Сервер не запустился: {e}"), Instant::now()));
            } else {
                crate::note(&format!("[ui] сервер запущен, порт {port}"));
            }
            let monitors_found = rat_session::capture::list_outputs().unwrap_or_default();
            crate::note(&format!("[ui] мониторов найдено: {}", monitors_found.len()));
            let monitor_sel = args.monitor;
            server.monitor.store(monitor_sel, Ordering::Relaxed);
            let mut device = load_device();
            if let Some(id) = &args.device_id {
                device.id = id.clone();
            }
            if let Some(relay) = &args.relay {
                device.relay = normalize_relay(relay);
                device.mode = ConnectionMode::Internet;
            }
            device.relay = normalize_relay(&device.relay);
            if !is_valid_id(&device.id) {
                match gen_id() {
                    Ok(id) => device.id = id,
                    Err(error) => {
                        crate::note(&format!("[ui] не удалось создать ID: {error}"));
                        device.id.clear();
                        if toast.is_none() {
                            toast =
                                Some((format!("Не удалось создать ID: {error}"), Instant::now()));
                        }
                    }
                }
            }
            save_device(&device);
            let settings_mode = device.mode;
            let settings_relay_edit = device.relay.clone();
            let settings_auto_allow = device.auto_allow;
            crate::approval::set_auto_allow(device.auto_allow);
            // Значок в трее создаём всегда — в двух вариантах: помощник службы
            // и обычный (ручной) запуск. Если оболочка ещё не готова (старт до
            // входа пользователя), попытки повторяются в цикле UI.
            let tray_kind = if args.service_helper {
                crate::tray::Kind::Service
            } else {
                crate::tray::Kind::Manual
            };
            let serve_running = server.is_running();
            let mut tray_retry_left: u32 = 0;
            let mut tray_retry_at = Instant::now();
            // Пока проводник (панель задач) не поднялся, значок не создаём:
            // неудачная попытка до старта панели задач оставляет служебное
            // окно, которое при рассылке TaskbarCreated «оживает» вторым
            // значком — в трее появлялся дубликат «Rat Next — служба».
            let tray = if !crate::shell_ready() {
                crate::note("[ui] оболочка не готова — значок отложен");
                if args.smoke.is_none() {
                    tray_retry_left = 120;
                    tray_retry_at = Instant::now() + Duration::from_secs(5);
                }
                None
            } else {
                match crate::tray::Tray::new(tray_kind) {
                    Ok(tray) => {
                        crate::note("[ui] значок в трее создан");
                        Some(tray)
                    }
                    Err(e) => {
                        crate::note(&format!("[ui] значок в трее не создан: {e}"));
                        // см. tray::cleanup_orphans: неудачная попытка могла
                        // оставить «пустое» окно значка.
                        crate::tray::cleanup_orphans();
                        if toast.is_none() {
                            toast = Some((
                                format!("Не удалось создать значок в трее: {e}"),
                                Instant::now(),
                            ));
                        }
                        if args.smoke.is_none() {
                            tray_retry_left = 120;
                            tray_retry_at = Instant::now() + Duration::from_secs(5);
                        }
                        None
                    }
                }
            };
            let mut internet = InternetHandle::new(server.monitor.clone(), incoming_tx);
            if device.mode == ConnectionMode::Internet && is_valid_id(&device.id) {
                let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "rat-pc".into());
                internet.start(device.relay.clone(), device.id.clone(), name);
            }
            Self {
                machines,
                statuses,
                server,
                internet,
                device,
                settings_open: false,
                settings_mode,
                settings_relay_edit,
                settings_auto_allow,
                settings_error: None,
                settings_serve_local: serve_running,
                settings_serve_port: port.to_string(),
                incoming_rx,
                incoming_pending: None,
                my_ip: local_ip(),
                my_name: std::env::var("COMPUTERNAME").unwrap_or_default(),
                screen: Screen::Home,
                viewer: None,
                files: None,
                title_now: "Rat Next".to_string(),
                connect_rx: None,
                connecting_to: None,
                toast,
                add_open: false,
                add_name: String::new(),
                add_addr: String::new(),
                add_err: None,
                edit_open: None,
                edit_name: String::new(),
                autoconnect_done: false,
                files_ui_done: false,
                started: Instant::now(),
                finished: false,
                tray,
                tray_retry_left,
                tray_retry_at,
                svc_detached: false,
                svc_probe_checked: Instant::now(),
                quit_at: None,
                quitting: false,
                hidden: false,
                tray_hide_done: false,
                shown_for_dialog: false,
                elevated: rat_session::elevation::is_elevated(),
                #[cfg(feature = "ffmpeg")]
                svc_state: crate::service::State::NotInstalled,
                #[cfg(feature = "ffmpeg")]
                svc_checked: Instant::now(),
                show_request,
                quit_request,
                repaint_slot,
                args,
            }
        }

        fn toast(&mut self, s: impl Into<String>) {
            self.toast = Some((s.into(), Instant::now()));
        }

        /// Открыть окно настроек (поля синхронизируются с фактическим состоянием).
        fn open_settings(&mut self) {
            self.settings_mode = self.device.mode;
            self.settings_relay_edit = self.device.relay.clone();
            self.settings_auto_allow = self.device.auto_allow;
            self.settings_serve_local = self.server.is_running();
            self.settings_serve_port = self.server.port.to_string();
            self.settings_error = None;
            self.settings_open = true;
        }

        /// Строка состояния главного окна: (цвет точки, текст).
        fn status_line(&self) -> (egui::Color32, String) {
            if self.device.mode == ConnectionMode::Internet {
                let st = self
                    .internet
                    .status
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_default();
                if !self.internet.is_running() {
                    (OFF, "Интернет (relay) выключен".to_string())
                } else if st.starts_with("ошибка") {
                    (OFF, format!("Нет связи с relay: {st}"))
                } else if st.starts_with("подключаюсь") || st.starts_with("соединение закрыто")
                {
                    (DIM2, "Подключаюсь к relay…".to_string())
                } else if st.starts_with("сессия") {
                    (ON, "Сеанс с оператором (через relay)".to_string())
                } else if st.starts_with("ожидаю") {
                    (
                        ON,
                        "На связи через интернет (relay) — жду подключения".to_string(),
                    )
                } else {
                    (DIM2, format!("Интернет (relay): {st}"))
                }
            } else if self.server.is_running() {
                (
                    ON,
                    format!("Локальная сеть — приём на порту {}", self.server.port),
                )
            } else {
                (DIM2, "Локальная сеть — приём выключен".to_string())
            }
        }

        fn start_connect(&mut self, name: String, addr: String) {
            if self.viewer.is_some() || self.connect_rx.is_some() {
                return;
            }
            self.connecting_to = Some(addr.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            let addr_conn = addr.clone();
            let name_conn = name.clone();
            let switch_after = self.args.switch_after;
            let relay_conn = self.device.relay.clone();
            let internet = self.device.mode == ConnectionMode::Internet;
            std::thread::spawn(move || {
                let mut connected = None;
                let mut last_err = String::new();
                for attempt in 0..10 {
                    let r = if internet {
                        if relay_conn.trim().is_empty() {
                            Err(std::io::Error::other("не задан сервер (Интернет-режим)"))
                        } else {
                            rat_session::link::relay_connect(relay_conn.as_str(), &addr_conn)
                        }
                    } else {
                        tcp_connect(addr_conn.as_str())
                    };
                    match r {
                        Ok(v) => {
                            connected = Some(v);
                            break;
                        }
                        Err(e) => {
                            last_err = e.to_string();
                            if attempt < 9 {
                                std::thread::sleep(Duration::from_millis(300));
                            }
                        }
                    }
                }
                let res = match connected {
                    Some((link, closer)) => {
                        let stats = Arc::new(Mutex::new(SessionStats::default()));
                        let stop = Arc::new(AtomicBool::new(false));
                        let base = TimeBase(Instant::now());
                        let (ui_tx, ui_rx) = sync_channel::<FrameOut>(3);
                        let input_q: Arc<Mutex<Vec<InputEvent>>> = Arc::new(Mutex::new(Vec::new()));
                        let cursor = Arc::new(Mutex::new(CursorState::default()));
                        let info: Arc<Mutex<Option<SessionInfo>>> = Arc::new(Mutex::new(None));
                        let monitors: Arc<Mutex<Vec<(u16, u16, u16)>>> =
                            Arc::new(Mutex::new(Vec::new()));
                        let pending_switch: Arc<Mutex<Option<u16>>> = Arc::new(Mutex::new(None));
                        let clip_in: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
                        let pending_paste: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
                        let cursor_img: Arc<Mutex<Option<rat_session::viewer::CursorImage>>> =
                            Arc::new(Mutex::new(None));
                        let cursor_img_ver = Arc::new(std::sync::atomic::AtomicU64::new(0));
                        let handle = spawn_viewer(
                            base,
                            stop.clone(),
                            stats.clone(),
                            link,
                            ui_tx,
                            ViewerShared {
                                input_q: input_q.clone(),
                                cursor: cursor.clone(),
                                info: info.clone(),
                                monitors: monitors.clone(),
                                pending_switch: pending_switch.clone(),
                                clip_in: clip_in.clone(),
                                pending_paste: pending_paste.clone(),
                                cursor_img: cursor_img.clone(),
                                cursor_img_ver: cursor_img_ver.clone(),
                            },
                        );
                        Ok(ViewerSession {
                            name: name_conn,
                            addr: addr_conn,
                            rx: ui_rx,
                            stats,
                            stop,
                            handle: Some(handle),
                            closer,
                            base,
                            texture: None,
                            last: None,
                            input_q,
                            cursor,
                            info,
                            monitors,
                            pending_switch,
                            clip_in,
                            pending_paste,
                            last_clip_applied: None,
                            top_open: false,
                            bottom_open: false,
                            top_t: 0.0,
                            bottom_t: 0.0,
                            pinned: false,
                            switch_after,
                            switch_done: false,
                            connected_at: Instant::now(),
                            last_pointer: None,
                            last_cursor_pos: None,
                            last_cursor_change_at: Instant::now() - Duration::from_secs(10),
                            last_move_at: Instant::now() - Duration::from_secs(10),
                            prev_mods: egui::Modifiers::default(),
                            buttons_down: [false; 5],
                            ui_e2e_ms: Vec::new(),
                            cursor_img,
                            cursor_img_ver,
                            cursor_tex: None,
                            cursor_tex_ver: u64::MAX,
                        })
                    }
                    None => Err(format!("не удалось подключиться к {addr_conn}: {last_err}")),
                };
                let _ = tx.send(res);
            });
            self.connect_rx = Some(rx);
        }

        fn poll_connect(&mut self) {
            let Some(rx) = self.connect_rx.as_ref() else {
                return;
            };
            let res = rx.try_recv();
            match res {
                Ok(Ok(session)) => {
                    self.connecting_to = None;
                    self.connect_rx = None;
                    crate::note(&format!("[ui] подключено: {}", session.addr));
                    {
                        let mut list = self.machines.lock().unwrap();
                        if let Some(m) = list.iter_mut().find(|m| m.addr == session.addr) {
                            m.last_connected = Some(now_unix());
                        }
                        save(&list);
                    }
                    self.viewer = Some(session);
                    self.screen = Screen::Viewer;
                }
                Ok(Err(e)) => {
                    self.connecting_to = None;
                    self.connect_rx = None;
                    crate::note(&format!("[ui] ошибка подключения: {e}"));
                    self.toast(e);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.connecting_to = None;
                    self.connect_rx = None;
                }
            }
        }

        fn home_ui(&mut self, ui: &mut egui::Ui) {
            self.set_title(ui.ctx(), "Rat Next");
            egui::Panel::top("top").show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let (st_col, st_text) = self.status_line();
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 5.0, st_col);
                    ui.label(st_text);
                    if let Some(e) = self.server.last_err.lock().unwrap().clone() {
                        ui.separator();
                        ui.colored_label(OFF, format!("ошибка: {e}"));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Настройки").clicked() {
                            self.open_settings();
                        }
                    });
                });
                // «ID: <номер>» — слева, сразу под строкой состояния; копирование — иконкой.
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("ID:")
                            .size(26.0)
                            .color(egui::Color32::from_rgb(58, 63, 70)),
                    );
                    let id_font = egui::FontId::new(26.0, egui::FontFamily::Name(ID_BOLD.into()));
                    ui.label(
                        egui::RichText::new(self.device.id.clone())
                            .font(id_font)
                            .color(egui::Color32::from_rgb(31, 35, 40)),
                    );
                    if copy_icon_button(ui)
                        .on_hover_text("скопировать ID")
                        .clicked()
                    {
                        ui.ctx().copy_text(self.device.id.clone());
                        self.toast("ID скопирован в буфер обмена");
                    }
                });
                if !self.elevated && !self.args.service_helper {
                    ui.horizontal(|ui| {
                        ui.colored_label(OFF, "⚠");
                        ui.label(
                            egui::RichText::new(
                                "нет прав администратора — управление окнами программ, \
                                 запущенных от администратора, недоступно",
                            )
                            .size(12.0)
                            .color(egui::Color32::from_rgb(180, 83, 9)),
                        )
                        .on_hover_text(
                            "Перезапустите от имени администратора, включите автозапуск \
                             (задача с повышенными правами) или установите службу — \
                             тогда мышь и клавиатура работают в любых окнах.",
                        );
                    });
                }
                ui.add_space(6.0);
            });
            // Нижняя строка — только для временных сообщений (идёт подключение, тост).
            let has_bottom = self.connecting_to.is_some()
                || self
                    .toast
                    .as_ref()
                    .is_some_and(|(_, at)| at.elapsed().as_secs_f32() < 6.0);
            if has_bottom {
                egui::Panel::bottom("bottom").show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if let Some(a) = &self.connecting_to {
                            ui.spinner();
                            ui.label(format!("Подключение к {a}…"));
                        }
                        if let Some((msg, at)) = &self.toast
                            && at.elapsed().as_secs_f32() < 6.0
                        {
                            ui.colored_label(egui::Color32::from_rgb(255, 200, 80), msg);
                        }
                    });
                    ui.add_space(4.0);
                });
            }

            let snapshot: Vec<(usize, Machine, bool)> = {
                let list = self.machines.lock().unwrap();
                let st = self.statuses.lock().unwrap();
                list.iter()
                    .enumerate()
                    .map(|(i, m)| (i, m.clone(), st.get(&m.addr).copied().unwrap_or(false)))
                    .collect()
            };
            let mut actions: Vec<TileAction> = Vec::new();
            egui::CentralPanel::default().show(ui, |ui| {
                if snapshot.is_empty() {
                    ui.add_space(16.0);
                    ui.vertical_centered(|ui| {
                        ui.label("Пока нет машин.");
                        ui.label("Нажмите «+ Добавить машину», чтобы добавить первую.");
                        ui.label("Подключение — по адресу IP:порт (например 192.168.1.50:55140).");
                    });
                    ui.add_space(16.0);
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        // Плитка «+ Добавить машину» — всегда первая в списке.
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(225.0, 112.0), egui::Sense::click());
                        let border = if resp.hovered() {
                            CARD_BORDER_HOVER
                        } else {
                            CARD_BORDER
                        };
                        let painter = ui.painter();
                        painter.rect_filled(rect, 8.0, border);
                        painter.rect_filled(rect.shrink(1.5), 7.0, ADD_CARD_BG);
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "+ Добавить машину",
                            egui::FontId::proportional(14.0),
                            DIM,
                        );
                        if resp.clicked() {
                            actions.push(TileAction::Add);
                        }
                        for (i, m, online) in &snapshot {
                            let addr = m.addr.clone();
                            let (resp, files_clicked) = tile(ui, m, *online);
                            if files_clicked {
                                actions.push(TileAction::Files(*i));
                            } else if resp.clicked() {
                                actions.push(TileAction::Connect(*i));
                            }
                            resp.context_menu(|ui| {
                                if ui.button("Подключиться").clicked() {
                                    actions.push(TileAction::Connect(*i));
                                    ui.close();
                                }
                                if ui.button("Файлы").clicked() {
                                    actions.push(TileAction::Files(*i));
                                    ui.close();
                                }
                                if ui.button("Переименовать").clicked() {
                                    actions.push(TileAction::Rename(*i));
                                    ui.close();
                                }
                                if ui.button("Скопировать адрес").clicked() {
                                    ui.ctx().copy_text(addr.clone());
                                    ui.close();
                                }
                                if ui.button("Удалить").clicked() {
                                    actions.push(TileAction::Delete(*i));
                                    ui.close();
                                }
                            });
                        }
                    });
                });
            });
            for a in actions {
                match a {
                    TileAction::Connect(i) => {
                        let pair = {
                            let list = self.machines.lock().unwrap();
                            list.get(i).map(|m| (m.name.clone(), m.addr.clone()))
                        };
                        if let Some((name, addr)) = pair {
                            self.start_connect(name, addr);
                        }
                    }
                    TileAction::Files(i) => {
                        let pair = {
                            let list = self.machines.lock().unwrap();
                            list.get(i).map(|m| (m.name.clone(), m.addr.clone()))
                        };
                        if let Some((name, addr)) = pair {
                            self.open_files(name, addr);
                        }
                    }
                    TileAction::Rename(i) => {
                        let name = {
                            let list = self.machines.lock().unwrap();
                            list.get(i).map(|m| m.name.clone())
                        };
                        if let Some(n) = name {
                            self.edit_name = n;
                            self.edit_open = Some(i);
                        }
                    }
                    TileAction::Delete(i) => {
                        let mut list = self.machines.lock().unwrap();
                        if i < list.len() {
                            list.remove(i);
                        }
                        save(&list);
                    }
                    TileAction::Add => {
                        self.add_open = true;
                        self.add_err = None;
                    }
                }
            }
        }

        fn dialogs(&mut self, ctx: &egui::Context) {
            if self.incoming_pending.is_none()
                && let Ok(request) = self.incoming_rx.try_recv()
            {
                self.incoming_pending = Some(request);
                crate::notify::beep();
                // Если окно скрыто в трее — показать его, иначе запрос не увидеть.
                if self.hidden {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.hidden = false;
                    self.shown_for_dialog = true;
                }
            }
            let request_expired = self
                .incoming_pending
                .as_ref()
                .is_some_and(|request| request.expires_at <= Instant::now());
            if request_expired && let Some(request) = self.incoming_pending.take() {
                let _ = request.reply.send(false);
            }

            let mut approval_decision = None;
            if let Some(request) = self.incoming_pending.as_ref() {
                let mut open = true;
                egui::Window::new("Подтверждение подключения")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .open(&mut open)
                    .show(ctx, |ui| {
                        let source = match request.source {
                            Source::Lan => "локальной сети",
                            Source::Internet => "Интернета через relay",
                        };
                        ui.label(format!("Запрос на подключение из {source}."));
                        ui.label("Будет открыт удалённый просмотр экрана и управление.");
                        ui.label("Разрешить эту сессию?");
                        ui.horizontal(|ui| {
                            if ui.button("Разрешить").clicked() {
                                approval_decision = Some(true);
                            }
                            if ui.button("Отклонить").clicked() {
                                approval_decision = Some(false);
                            }
                        });
                    });
                if !open && approval_decision.is_none() {
                    approval_decision = Some(false);
                }
            }
            if let Some(allowed) = approval_decision
                && let Some(request) = self.incoming_pending.take()
            {
                let _ = request.reply.send(allowed);
            }
            // Если окно разбудили только ради подтверждения — вернуть его в трей.
            if self.incoming_pending.is_none() && self.shown_for_dialog {
                self.shown_for_dialog = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.hidden = true;
            }

            if self.settings_open {
                // Служба: состояние опрашиваем не чаще раза в секунду.
                #[cfg(feature = "ffmpeg")]
                if self.svc_checked.elapsed() > std::time::Duration::from_secs(1) {
                    self.svc_checked = Instant::now();
                    self.svc_state = crate::service::status();
                }
                let mut open = true;
                let mut apply = false;
                let mut cancel = false;
                egui::Window::new("Настройки")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.set_min_width(430.0);
                        ui.label(egui::RichText::new("Этот компьютер").strong());
                        ui.label(format!(
                            "{} · ID: {}",
                            if self.my_name.is_empty() {
                                "без имени"
                            } else {
                                self.my_name.as_str()
                            },
                            self.device.id
                        ));
                        ui.label(
                            egui::RichText::new(format!(
                                "Локальный адрес: {}:{}",
                                self.my_ip, self.server.port
                            ))
                            .small()
                            .color(egui::Color32::from_gray(150)),
                        );
                        ui.separator();
                        ui.label(egui::RichText::new("Как запускаться").strong());
                        ui.label(
                            "Служба Windows — приложение включается само при загрузке ПК \
                             и доступно до входа пользователя.",
                        );
                        let (state_txt, state_col) = match self.svc_state {
                            crate::service::State::NotInstalled => {
                                ("не установлена", egui::Color32::from_gray(130))
                            }
                            crate::service::State::Stopped => {
                                ("остановлена", egui::Color32::from_rgb(230, 120, 30))
                            }
                            crate::service::State::Running => ("работает", ON),
                            crate::service::State::Pending => {
                                ("меняется…", egui::Color32::from_rgb(150, 150, 60))
                            }
                        };
                        ui.horizontal(|ui| {
                            ui.label("Состояние:");
                            ui.colored_label(state_col, state_txt);
                        });
                        let not_installed =
                            matches!(self.svc_state, crate::service::State::NotInstalled);
                        let running = matches!(self.svc_state, crate::service::State::Running);
                        let pending = matches!(self.svc_state, crate::service::State::Pending);
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    not_installed && !pending,
                                    egui::Button::new("Установить"),
                                )
                                .clicked()
                            {
                                self.svc_action(crate::service::Op::Install);
                            }
                            // Одна кнопка пуск/стоп — меняет надпись по состоянию.
                            let toggle = if running {
                                Some(("Остановить", crate::service::Op::Stop))
                            } else if !not_installed && !pending {
                                Some(("Запустить", crate::service::Op::Start))
                            } else {
                                None
                            };
                            if let Some((label, op)) = toggle {
                                if ui.button(label).clicked() {
                                    self.svc_action(op);
                                }
                            } else {
                                ui.add_enabled(
                                    false,
                                    egui::Button::new(if running {
                                        "Остановить"
                                    } else {
                                        "Запустить"
                                    }),
                                );
                            }
                            if ui
                                .add_enabled(
                                    !not_installed && !pending,
                                    egui::Button::new("Удалить"),
                                )
                                .clicked()
                            {
                                self.svc_action(crate::service::Op::Uninstall);
                            }
                        });
                        ui.label(
                            egui::RichText::new(
                                "Без службы приложение работает, только пока вы его запустили \
                                 (ярлыком). Постоянный доступ — включите службу.",
                            )
                            .small()
                            .color(egui::Color32::from_gray(150)),
                        );
                        ui.separator();
                        ui.label(egui::RichText::new("Подключение").strong());
                        ui.radio_value(
                            &mut self.settings_mode,
                            ConnectionMode::Lan,
                            "Локальная сеть (IP:порт)",
                        );
                        ui.radio_value(
                            &mut self.settings_mode,
                            ConnectionMode::Internet,
                            "Интернет (через relay)",
                        );
                        if self.settings_mode == ConnectionMode::Internet {
                            ui.horizontal(|ui| {
                                ui.label("Адрес relay:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.settings_relay_edit)
                                        .desired_width(230.0),
                                );
                            });
                        }
                        ui.label(
                            egui::RichText::new(
                                "Локальная сеть: подключаемся по адресу вида 192.168.1.50:55140 \
                                 (или адрес Radmin VPN). Интернет: по 6-значному ID через relay.",
                            )
                            .small()
                            .color(egui::Color32::from_gray(150)),
                        );
                        ui.separator();
                        ui.label(egui::RichText::new("Безопасность").strong());
                        ui.checkbox(
                            &mut self.settings_auto_allow,
                            "Подключать без подтверждения (и в локальной сети, и через relay)",
                        );
                        ui.separator();
                        ui.label(egui::RichText::new("Служебное").strong());
                        ui.checkbox(
                            &mut self.settings_serve_local,
                            "Принимать подключения по локальной сети (порт 55140)",
                        );
                        if self.settings_serve_local {
                            ui.horizontal(|ui| {
                                ui.label("Порт:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.settings_serve_port)
                                        .desired_width(56.0),
                                );
                            });
                        }
                        ui.label(
                            egui::RichText::new(
                                "Работает независимо от режима подключения. Нужен только для \
                                 прямых подключений по сети; на интернет-режим (relay) не влияет.",
                            )
                            .small()
                            .color(egui::Color32::from_gray(150)),
                        );

                        if let Some(error) = &self.settings_error {
                            ui.colored_label(OFF, error);
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Сохранить").clicked() {
                                apply = true;
                            }
                            if ui.button("Отмена").clicked() {
                                cancel = true;
                            }
                        });
                    });
                if apply {
                    let relay = normalize_relay(&self.settings_relay_edit);
                    let valid_relay = relay.rsplit_once(':').is_some_and(|(host, port)| {
                        !host.trim().is_empty()
                            && port.parse::<u16>().is_ok_and(|parsed| parsed > 0)
                    });
                    let serve_port = self.settings_serve_port.trim().parse::<u16>().ok();
                    let serve_ok = !self.settings_serve_local || serve_port.is_some_and(|p| p > 0);
                    if !serve_ok {
                        self.settings_error =
                            Some("Некорректный порт локального приёма (1–65535)".to_string());
                    } else if self.settings_mode == ConnectionMode::Internet && !valid_relay {
                        self.settings_error = Some(
                            "Укажите адрес relay в формате host:порт, например 194.37.254.233:8889"
                                .to_string(),
                        );
                    } else {
                        let old_mode = self.device.mode;
                        let old_relay = self.device.relay.clone();
                        let restart = old_mode != self.settings_mode
                            || old_relay != relay
                            || !self.internet.is_running();
                        self.device.mode = self.settings_mode;
                        self.device.relay = relay;
                        self.device.auto_allow = self.settings_auto_allow;
                        crate::approval::set_auto_allow(self.settings_auto_allow);
                        // Локальный приём — независимо от режима подключения.
                        let want_serve = self.settings_serve_local;
                        let want_port = serve_port.unwrap_or(self.server.port);
                        let serve_running = self.server.is_running();
                        if want_serve && (!serve_running || self.server.port != want_port) {
                            self.server.stop();
                            if let Err(e) = self.server.start(want_port) {
                                self.settings_serve_local = false;
                                self.toast(format!("Приём подключений не запустился: {e}"));
                            }
                        } else if !want_serve && serve_running {
                            self.server.stop();
                            self.toast("Локальный приём выключен");
                        }
                        save_device(&self.device);
                        self.settings_open = false;
                        self.settings_error = None;
                        if restart {
                            self.internet.stop();
                        }
                        if self.device.mode == ConnectionMode::Internet && restart {
                            if is_valid_id(&self.device.id) {
                                let name = if self.my_name.is_empty() {
                                    "rat-pc".to_string()
                                } else {
                                    self.my_name.clone()
                                };
                                self.internet.start(
                                    self.device.relay.clone(),
                                    self.device.id.clone(),
                                    name,
                                );
                            } else {
                                self.toast("Не удалось создать шестизначный ID");
                            }
                        }
                    }
                } else if cancel || !open {
                    self.settings_open = false;
                    self.settings_error = None;
                }
            }

            if self.add_open {
                let mut open = self.add_open;
                egui::Window::new("Добавить машину")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label("Имя машины:");
                        ui.add(egui::TextEdit::singleline(&mut self.add_name).desired_width(280.0));
                        let address_hint = match self.device.mode {
                            ConnectionMode::Internet => "ID устройства (6 цифр):",
                            ConnectionMode::Lan => "Адрес машины (IP:порт):",
                        };
                        ui.label(address_hint);
                        ui.add(egui::TextEdit::singleline(&mut self.add_addr).desired_width(280.0));
                        if let Some(e) = &self.add_err {
                            ui.colored_label(OFF, e);
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Добавить").clicked() {
                                let addr = match self.device.mode {
                                    ConnectionMode::Internet => self.add_addr.trim().to_string(),
                                    ConnectionMode::Lan => normalize_addr(&self.add_addr),
                                };
                                let ok = match self.device.mode {
                                    ConnectionMode::Internet => is_valid_id(&addr),
                                    ConnectionMode::Lan => {
                                        let (host, port) = split_addr(&addr);
                                        !host.is_empty() && port.is_some_and(|value| value > 0)
                                    }
                                };
                                if !ok {
                                    self.add_err = Some(match self.device.mode {
                                        ConnectionMode::Internet => {
                                            "Введите ID ровно из 6 цифр".into()
                                        }
                                        ConnectionMode::Lan => {
                                            "Введите адрес машины в формате IP:порт".into()
                                        }
                                    });
                                } else {
                                    let mut list = self.machines.lock().unwrap();
                                    if list.iter().any(|m| m.addr == addr) {
                                        self.add_err =
                                            Some("Такая машина уже есть в списке".into());
                                    } else {
                                        let name = if self.add_name.trim().is_empty() {
                                            addr.clone()
                                        } else {
                                            self.add_name.trim().to_string()
                                        };
                                        list.push(Machine {
                                            name,
                                            addr,
                                            last_connected: None,
                                        });
                                        save(&list);
                                        drop(list);
                                        self.add_err = None;
                                        self.add_open = false;
                                        self.add_name.clear();
                                        self.add_addr.clear();
                                    }
                                }
                            }
                            if ui.button("Отмена").clicked() {
                                self.add_open = false;
                            }
                        });
                    });
                self.add_open = open && self.add_open;
            }
            if let Some(i) = self.edit_open {
                let mut open = true;
                egui::Window::new("Переименовать машину")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_name).desired_width(280.0),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Сохранить").clicked() {
                                {
                                    let mut list = self.machines.lock().unwrap();
                                    if let Some(m) = list.get_mut(i) {
                                        let nm = self.edit_name.trim();
                                        if !nm.is_empty() {
                                            m.name = nm.to_string();
                                        }
                                    }
                                    save(&list);
                                }
                                self.edit_open = None;
                            }
                            if ui.button("Отмена").clicked() {
                                self.edit_open = None;
                            }
                        });
                    });
                if !open {
                    self.edit_open = None;
                }
            }
        }
    }

    /// Кнопка-иконка «копировать» — два перекрывающихся листа, без надписи.
    fn copy_icon_button(ui: &mut egui::Ui) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(31.0, 28.0), egui::Sense::click());
        let bg = if resp.hovered() {
            egui::Color32::from_rgb(238, 241, 246)
        } else {
            egui::Color32::from_rgb(250, 251, 253)
        };
        let ink = egui::Color32::from_rgb(90, 97, 110);
        let painter = ui.painter();
        painter.rect_filled(rect, 6.0, CARD_BORDER);
        painter.rect_filled(rect.shrink(1.0), 5.0, bg);
        let c = rect.center();
        let back = egui::Rect::from_min_size(c + egui::vec2(-1.0, -8.0), egui::vec2(10.5, 12.0));
        let front = egui::Rect::from_min_size(c + egui::vec2(-7.5, -4.0), egui::vec2(10.5, 12.0));
        painter.rect_filled(back, 1.8, ink);
        painter.rect_filled(back.shrink(1.3), 1.0, bg);
        painter.rect_filled(front, 1.8, ink);
        painter.rect_filled(front.shrink(1.3), 1.0, bg);
        resp
    }

    fn tile(ui: &mut egui::Ui, m: &Machine, online: bool) -> (egui::Response, bool) {
        let desired = egui::vec2(225.0, 112.0);
        let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
        let border = if resp.hovered() {
            CARD_BORDER_HOVER
        } else {
            CARD_BORDER
        };
        let painter = ui.painter();
        painter.rect_filled(rect, 8.0, border);
        painter.rect_filled(rect.shrink(1.5), 7.0, CARD_BG);
        painter.circle_filled(
            egui::pos2(rect.left() + 18.0, rect.top() + 20.0),
            5.0,
            if online { ON } else { TILE_OFF_DOT },
        );
        painter.text(
            egui::pos2(rect.left() + 32.0, rect.top() + 11.0),
            egui::Align2::LEFT_TOP,
            &m.name,
            egui::FontId::proportional(16.0),
            TILE_TEXT,
        );
        painter.text(
            egui::pos2(rect.left() + 18.0, rect.top() + 40.0),
            egui::Align2::LEFT_TOP,
            format!("ID {}", m.addr),
            egui::FontId::monospace(12.0),
            DIM,
        );
        painter.text(
            egui::pos2(rect.left() + 18.0, rect.top() + 60.0),
            egui::Align2::LEFT_TOP,
            if online {
                "в сети"
            } else {
                "не в сети"
            },
            egui::FontId::proportional(12.5),
            if online { ON } else { DIM2 },
        );
        // Кнопки-подсказки: клик по карточке = подключиться; «Файлы» — менеджер.
        let btn = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 18.0, rect.bottom() - 34.0),
            egui::vec2(100.0, 24.0),
        );
        painter.rect_filled(
            btn,
            6.0,
            if resp.hovered() {
                BTN_BLUE
            } else {
                BTN_BLUE_DIM
            },
        );
        painter.text(
            btn.center(),
            egui::Align2::CENTER_CENTER,
            "Подключиться",
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );
        let fbtn = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 124.0, rect.bottom() - 34.0),
            egui::vec2(58.0, 24.0),
        );
        let fresp = ui.interact(
            fbtn,
            egui::Id::new(("tile-files", &m.addr)),
            egui::Sense::click(),
        );
        painter.rect_filled(fbtn, 6.0, egui::Color32::from_rgb(195, 199, 207));
        painter.rect_filled(
            fbtn.shrink(1.0),
            5.5,
            if fresp.hovered() {
                egui::Color32::from_rgb(233, 238, 245)
            } else {
                egui::Color32::from_rgb(252, 252, 253)
            },
        );
        painter.text(
            fbtn.center(),
            egui::Align2::CENTER_CENTER,
            "Файлы",
            egui::FontId::proportional(12.5),
            egui::Color32::from_rgb(58, 63, 70),
        );
        (resp, fresp.clicked())
    }

    fn local_ip() -> String {
        std::net::UdpSocket::bind("0.0.0.0:0")
            .and_then(|s| {
                s.connect("8.8.8.8:80")?;
                s.local_addr().map(|a| a.ip().to_string())
            })
            .unwrap_or_else(|_| "?".into())
    }

    fn split_addr(s: &str) -> (String, Option<u16>) {
        match s.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().ok()),
            None => (s.to_string(), None),
        }
    }
    impl RatApp {
        /// Заголовок окна (меняется по экранам).
        fn set_title(&mut self, ctx: &egui::Context, t: &str) {
            if self.title_now != t {
                self.title_now = t.to_string();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(t.to_string()));
            }
        }

        fn open_files(&mut self, name: String, addr: String) {
            if self.viewer.is_some() || self.connect_rx.is_some() || self.files.is_some() {
                return;
            }
            let target = if self.device.mode == ConnectionMode::Internet && !addr.contains(':') {
                rat_session::files_worker::Target::Relay {
                    relay: self.device.relay.clone(),
                    id: addr.clone(),
                }
            } else {
                rat_session::files_worker::Target::Tcp(addr.clone())
            };
            self.files = Some(FilesState::open(name, addr, target));
            self.screen = Screen::Files;
        }

        fn close_files(&mut self) {
            if let Some(mut f) = self.files.take() {
                f.save_prefs();
                f.shutdown();
            }
            self.screen = Screen::Home;
        }

        // Типы менеджера файлов — на уровне модуля (см. выше impl RatApp).
        fn files_ui(&mut self, ui: &mut egui::Ui) {
            let title = self.files.as_ref().map(|f| format!("Файлы — {}", f.name));
            let Some(title) = title else {
                self.screen = Screen::Home;
                return;
            };
            self.set_title(ui.ctx(), &title);
            let close = {
                let Some(f) = self.files.as_mut() else {
                    return;
                };
                f.tick(ui.ctx());
                f.ui(ui);
                f.dialogs(ui.ctx());
                f.want_close
            };
            if close {
                self.close_files();
            }
        }

        fn viewer_ui(&mut self, ui: &mut egui::Ui) {
            let mut back = false;
            let Some(v) = self.viewer.as_mut() else {
                self.screen = Screen::Home;
                return;
            };
            loop {
                match v.rx.try_recv() {
                    Ok(f) => {
                        let now = rat_session::stats::now_us(&v.base);
                        let e = ((now as i64 - f.t_capture_us as i64).max(0)) as f32 / 1000.0;
                        SessionStats::push_sample(&mut v.ui_e2e_ms, e);
                        v.last = Some(f);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break,
                }
            }
            if let Some(f) = &v.last {
                let img = egui::ColorImage::from_rgba_unmultiplied([f.w, f.h], &f.rgba);
                let data = egui::ImageData::Color(std::sync::Arc::new(img));
                match &mut v.texture {
                    Some(t) => t.set(data, egui::TextureOptions::LINEAR),
                    None => {
                        v.texture = Some(ui.ctx().load_texture(
                            "video",
                            data,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
            }
            // Тестовый авто-переключатель монитора (--switch-after=СЕК).
            if let Some(sec) = v.switch_after
                && !v.switch_done
                && v.connected_at.elapsed().as_secs_f32() >= sec
            {
                v.switch_done = true;
                let ids: Vec<u16> = v.monitors.lock().unwrap().iter().map(|m| m.0).collect();
                if !ids.is_empty() {
                    let cur = v
                        .info
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|s| s.active)
                        .unwrap_or(ids[0]);
                    let next = ids.iter().copied().find(|&i| i != cur).unwrap_or(cur);
                    *v.pending_switch.lock().unwrap() = Some(next);
                    crate::note(&format!("[ui] авто-переключение монитора -> {next}"));
                }
            }
            // Локальный буфер: текст, скопированный на удалённой машине.
            let clip_now = v.clip_in.lock().unwrap().clone();
            if clip_now != v.last_clip_applied {
                if let Some(text) = &clip_now {
                    ui.ctx().copy_text(text.clone());
                }
                v.last_clip_applied = clip_now;
            }
            // ---- панели-оверлеи: прячутся, выезжают при наведении к краю ----
            const TOP_H: f32 = 42.0;
            const BOT_H: f32 = 30.0;
            const EDGE: f32 = 6.0;
            let screen = ui.ctx().content_rect();
            let (ptr0, any_down, dt) = ui.ctx().input(|i| {
                (
                    i.pointer.hover_pos(),
                    i.pointer.any_down(),
                    i.stable_dt.clamp(0.001, 0.05),
                )
            });
            let in_win = |p: egui::Pos2| screen.expand(4.0).contains(p);
            let want_top = v.pinned
                || (!any_down && ptr0.is_some_and(|p| in_win(p) && p.y <= screen.top() + EDGE))
                || (v.top_open
                    && ptr0.is_some_and(|p| in_win(p) && p.y <= screen.top() + TOP_H + 4.0));
            let want_bottom = v.pinned
                || (!any_down && ptr0.is_some_and(|p| in_win(p) && p.y >= screen.bottom() - EDGE))
                || (v.bottom_open
                    && ptr0.is_some_and(|p| in_win(p) && p.y >= screen.bottom() - BOT_H - 4.0));
            v.top_open = want_top;
            v.bottom_open = want_bottom;
            for (t, open, dur_in, dur_out) in [
                (&mut v.top_t, v.top_open, 0.35f32, 0.20f32),
                (&mut v.bottom_t, v.bottom_open, 0.30f32, 0.18f32),
            ] {
                let target = if open { 1.0 } else { 0.0 };
                let dur = if target > *t { dur_in } else { dur_out };
                let step = dt / dur;
                *t = if (target - *t).abs() <= step {
                    target
                } else {
                    *t + (target - *t).signum() * step
                };
            }
            // Светлая палитра — как у остального интерфейса: тёмный текст на
            // светлом фоне читается и поверх чёрных полей, и поверх видео.
            let bar_fill = egui::Color32::from_rgba_unmultiplied(243, 245, 248, 240);
            let bar_border = egui::Color32::from_rgb(196, 203, 213);
            if v.top_t > 0.01 {
                let y = screen.top() - TOP_H * (1.0 - v.top_t);
                egui::Area::new(egui::Id::new("vt_top"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(egui::pos2(screen.left(), y))
                    .interactable(true)
                    .show(ui.ctx(), |ui| {
                        let bg = egui::Rect::from_min_size(
                            egui::pos2(screen.left(), y),
                            egui::vec2(screen.width(), TOP_H),
                        );
                        ui.painter().rect_filled(bg, 0.0, bar_fill);
                        ui.painter().rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(screen.left(), y + TOP_H - 1.0),
                                egui::pos2(screen.right(), y + TOP_H),
                            ),
                            0.0,
                            bar_border,
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(screen.width(), TOP_H),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    let disconnect = egui::Button::new(
                                        egui::RichText::new("⬅")
                                            .size(24.0)
                                            .color(egui::Color32::from_rgb(219, 56, 52)),
                                    )
                                    .fill(egui::Color32::from_rgb(255, 236, 234));
                                    if ui
                                        .add(disconnect)
                                        .on_hover_text("Отключиться от удалённого ПК")
                                        .clicked()
                                    {
                                        back = true;
                                    }
                                    ui.separator();
                                    ui.label(format!("{} ({})", v.name, v.addr));
                                    let st = v.stats.lock().unwrap();
                                    ui.separator();
                                    ui.label(format!("fps: {:.1}", st.fps));
                                    ui.separator();
                                    ui.label(format!("e2e {:.0}ms", SessionStats::avg(&st.e2e_ms)));
                                    ui.separator();
                                    ui.label(format!("sent {} | shown {}", st.sent_pkts, st.shown));
                                    if !st.last_error.is_empty() {
                                        ui.separator();
                                        ui.colored_label(OFF, format!("ошибка: {}", st.last_error));
                                    }
                                    ui.separator();
                                    let mons = v.monitors.lock().unwrap().clone();
                                    let cur = v
                                        .info
                                        .lock()
                                        .unwrap()
                                        .as_ref()
                                        .map(|s| s.active)
                                        .unwrap_or(0);
                                    if !mons.is_empty() {
                                        ui.label("🖥").on_hover_text("Монитор — переключение");
                                        for (idx, mw, mh) in &mons {
                                            let sel = cur == *idx;
                                            if ui
                                                .selectable_label(
                                                    sel,
                                                    format!("{} ({}×{})", idx + 1, mw, mh),
                                                )
                                                .clicked()
                                                && !sel
                                            {
                                                *v.pending_switch.lock().unwrap() = Some(*idx);
                                            }
                                        }
                                    } else if let Some(si) = v.info.lock().unwrap().as_ref() {
                                        ui.label(format!(
                                            "монитор {}×{}",
                                            si.out.width, si.out.height
                                        ));
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.add_space(10.0);
                                            let is_fs = ui
                                                .ctx()
                                                .input(|i| i.viewport().fullscreen)
                                                .unwrap_or(false);
                                            let (label, hint) = if is_fs {
                                                ("🗗", "Вернуть в окно")
                                            } else {
                                                ("⛶", "Во весь экран")
                                            };
                                            if ui.button(label).on_hover_text(hint).clicked() {
                                                ui.ctx().send_viewport_cmd(
                                                    egui::ViewportCommand::Fullscreen(!is_fs),
                                                );
                                            }
                                            if ui
                                                .selectable_label(v.pinned, "📌")
                                                .on_hover_text(
                                                    "Закрепить панели — не скрывать автоматически",
                                                )
                                                .clicked()
                                            {
                                                v.pinned = !v.pinned;
                                            }
                                        },
                                    );
                                });
                            },
                        );
                    });
            }
            if v.bottom_t > 0.01 {
                let y = screen.bottom() - BOT_H * v.bottom_t;
                egui::Area::new(egui::Id::new("vt_bottom"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(egui::pos2(screen.left(), y))
                    .interactable(true)
                    .show(ui.ctx(), |ui| {
                        let bg = egui::Rect::from_min_size(
                            egui::pos2(screen.left(), y),
                            egui::vec2(screen.width(), BOT_H),
                        );
                        ui.painter().rect_filled(bg, 0.0, bar_fill);
                        ui.painter().rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(screen.left(), y),
                                egui::pos2(screen.right(), y + 1.0),
                            ),
                            0.0,
                            bar_border,
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(screen.width(), BOT_H),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add_space(10.0);
                                ui.label(
                                    "Мышь — над видео; клавиатура — когда окно в фокусе. Панели выезжают при наведении к верхнему/нижнему краю. Кнопка со стрелкой ⬅ или крестик окна — отключиться.",
                                );
                            },
                        );
                    });
            }

            let mut video_rect: Option<egui::Rect> = None;
            egui::CentralPanel::default().show(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                ui.painter()
                    .rect_filled(rect, 2.0, egui::Color32::from_rgb(12, 14, 18));
                if let (Some(tex), Some(f)) = (&v.texture, &v.last) {
                    let img_ar = f.w as f32 / f.h as f32;
                    let rect_ar = rect.width() / rect.height().max(1.0);
                    let draw = if img_ar > rect_ar {
                        let h = rect.width() / img_ar;
                        egui::Rect::from_min_size(
                            egui::pos2(rect.min.x, rect.min.y + (rect.height() - h) / 2.0),
                            egui::vec2(rect.width(), h),
                        )
                    } else {
                        let w = rect.height() * img_ar;
                        egui::Rect::from_min_size(
                            egui::pos2(rect.min.x + (rect.width() - w) / 2.0, rect.min.y),
                            egui::vec2(w, rect.height()),
                        )
                    };
                    ui.painter().image(
                        tex.id(),
                        draw,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    video_rect = Some(draw);
                } else {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "ожидание кадров…",
                        egui::FontId::proportional(22.0),
                        egui::Color32::from_gray(150),
                    );
                }
            });

            // ---- ввод и удалённый курсор ----
            let input_q = v.input_q.clone();
            let cursor_arc = v.cursor.clone();
            if let Some(draw) = video_rect {
                let resp = ui.allocate_rect(draw, egui::Sense::click_and_drag());
                let mut locked = input_q.lock().unwrap();
                let focused = ui.ctx().input(|i| i.focused);

                // Панель поверх видео: пока указатель над ней, ввод в сессию не шлём.
                let over_bars = ptr0.is_some_and(|p| {
                    p.y <= screen.top() + TOP_H * v.top_t + 1.0
                        || p.y >= screen.bottom() - BOT_H * v.bottom_t - 1.0
                });

                // Указатель: движение, кнопки, колесо.
                let engaged = ((resp.hovered() || resp.dragged()) && !over_bars)
                    || v.buttons_down.iter().any(|d| *d);
                if engaged {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                    if !over_bars && let Some(p) = ui.ctx().input(|i| i.pointer.latest_pos()) {
                        let rel = p - draw.min;
                        let (nx, ny) =
                            input::ui_px_to_norm(rel.x, rel.y, draw.width(), draw.height());
                        if v.last_pointer != Some((nx, ny)) {
                            v.last_pointer = Some((nx, ny));
                            v.last_move_at = Instant::now();
                            locked.push(InputEvent::Move { x: nx, y: ny });
                        }
                    }
                    let (mx, my) = v.last_pointer.unwrap_or((32768, 32768));
                    let mut pb: Vec<(u8, bool, f32, f32)> = Vec::new();
                    let mut wheel = (0f32, 0f32);
                    ui.ctx().input(|inp| {
                        wheel = (inp.smooth_scroll_delta.x, inp.smooth_scroll_delta.y);
                        for ev in &inp.events {
                            if let egui::Event::PointerButton {
                                pos,
                                button,
                                pressed,
                                ..
                            } = ev
                            {
                                let idx = match button {
                                    egui::PointerButton::Primary => 0u8,
                                    egui::PointerButton::Secondary => 1,
                                    egui::PointerButton::Middle => 2,
                                    egui::PointerButton::Extra1 => 3,
                                    egui::PointerButton::Extra2 => 4,
                                };
                                let rel = *pos - draw.min;
                                pb.push((idx, *pressed, rel.x, rel.y));
                            }
                        }
                    });
                    for (idx, pressed, rx, ry) in pb {
                        let i = idx as usize;
                        if (resp.hovered() && !over_bars) || v.buttons_down[i] {
                            let (nx, ny) =
                                input::ui_px_to_norm(rx, ry, draw.width(), draw.height());
                            v.last_pointer = Some((nx, ny));
                            v.last_move_at = Instant::now();
                            v.buttons_down[i] = pressed;
                            locked.push(InputEvent::Button {
                                btn: idx,
                                down: pressed,
                                x: nx,
                                y: ny,
                            });
                        }
                    }
                    if resp.hovered() && !over_bars && (wheel.0.abs() > 0.5 || wheel.1.abs() > 0.5)
                    {
                        locked.push(InputEvent::Wheel {
                            dx: wheel.0.clamp(-32768.0, 32767.0) as i16,
                            dy: wheel.1.clamp(-32768.0, 32767.0) as i16,
                            x: mx,
                            y: my,
                        });
                    }
                }

                // Клавиатура: пока окно в фокусе.
                if focused {
                    let mut keys: Vec<(u16, bool)> = Vec::new();
                    let mut texts: Vec<String> = Vec::new();
                    let mut specials: Vec<u8> = Vec::new();
                    let mut paste_text: Option<String> = None;
                    ui.ctx().input(|inp| {
                        for ev in &inp.events {
                            match ev {
                                egui::Event::Key {
                                    key,
                                    pressed,
                                    modifiers,
                                    ..
                                } => {
                                    if let Some(vk) = vk_of_key(*key) {
                                        let shortcut =
                                            modifiers.ctrl || modifiers.alt || modifiers.mac_cmd;
                                        if shortcut || !is_char_vk(vk) {
                                            keys.push((vk, *pressed));
                                        }
                                    }
                                }
                                egui::Event::Text(t) => texts.push(t.clone()),
                                egui::Event::Copy => specials.push(1),
                                egui::Event::Cut => specials.push(2),
                                egui::Event::Paste(t) => {
                                    paste_text = Some(t.clone());
                                    specials.push(3);
                                }
                                _ => {}
                            }
                        }
                    });
                    for (vk, down) in keys {
                        locked.push(InputEvent::Key { vk, down });
                    }
                    for t in texts {
                        for ch in t.chars() {
                            if let Some((vk, sh, ct, al)) = input::vk_for_char(ch) {
                                if al {
                                    locked.push(InputEvent::Key {
                                        vk: 0x12,
                                        down: true,
                                    });
                                }
                                if ct {
                                    locked.push(InputEvent::Key {
                                        vk: 0x11,
                                        down: true,
                                    });
                                }
                                if sh {
                                    locked.push(InputEvent::Key {
                                        vk: 0x10,
                                        down: true,
                                    });
                                }
                                locked.push(InputEvent::Key { vk, down: true });
                                locked.push(InputEvent::Key { vk, down: false });
                                if sh {
                                    locked.push(InputEvent::Key {
                                        vk: 0x10,
                                        down: false,
                                    });
                                }
                                if ct {
                                    locked.push(InputEvent::Key {
                                        vk: 0x11,
                                        down: false,
                                    });
                                }
                                if al {
                                    locked.push(InputEvent::Key {
                                        vk: 0x12,
                                        down: false,
                                    });
                                }
                            }
                        }
                    }
                    // egui перехватывает Ctrl+C/X/V — транслируем на удалённую машину.
                    for s in specials {
                        let vk = match s {
                            1 => 0x43u16,
                            2 => 0x58,
                            _ => 0x56,
                        };
                        locked.push(InputEvent::Key { vk, down: true });
                        locked.push(InputEvent::Key { vk, down: false });
                    }
                    // Вставка: текст сначала отправим агенту (он положит его
                    // в свой буфер), клавиши Ctrl+V уже в очереди ввода.
                    if let Some(text) = paste_text
                        && !text.is_empty()
                    {
                        *v.pending_paste.lock().unwrap() = Some(text);
                    }
                    let m = ui.ctx().input(|i| i.modifiers);
                    let pm = v.prev_mods;
                    for (now_on, was_on, vk) in [
                        (m.shift, pm.shift, 0x10u16),
                        (m.ctrl, pm.ctrl, 0x11),
                        (m.alt, pm.alt, 0x12),
                        (m.mac_cmd, pm.mac_cmd, 0x5B),
                    ] {
                        if now_on && !was_on {
                            locked.push(InputEvent::Key { vk, down: true });
                        }
                        if !now_on && was_on {
                            locked.push(InputEvent::Key { vk, down: false });
                        }
                    }
                    v.prev_mods = m;
                }

                // Потеря фокуса: отпустить зажатое, чтобы не «залипло».
                if !focused {
                    if v.buttons_down.iter().any(|d| *d) {
                        for (idx, d) in v.buttons_down.iter_mut().enumerate() {
                            if *d {
                                *d = false;
                                let (x, y) = v.last_pointer.unwrap_or((32768, 32768));
                                locked.push(InputEvent::Button {
                                    btn: idx as u8,
                                    down: false,
                                    x,
                                    y,
                                });
                            }
                        }
                    }
                    let pm = v.prev_mods;
                    for (was_on, vk) in [
                        (pm.shift, 0x10u16),
                        (pm.ctrl, 0x11),
                        (pm.alt, 0x12),
                        (pm.mac_cmd, 0x5B),
                    ] {
                        if was_on {
                            locked.push(InputEvent::Key { vk, down: false });
                        }
                    }
                    v.prev_mods = egui::Modifiers::default();
                }
                drop(locked);

                // Удалённый курсор поверх видео — настоящей картинкой, как на
                // удалённой машине; пока картинки нет — векторная форма.
                let cs = *cursor_arc.lock().unwrap();
                if v.last_cursor_pos != Some((cs.x, cs.y)) {
                    v.last_cursor_pos = Some((cs.x, cs.y));
                    v.last_cursor_change_at = Instant::now();
                }
                if cs.visible {
                    // Шаг 1 (против «залипаний» на интернет-канале): пока
                    // управление идёт от нас, рисуем курсор по последней
                    // отправленной цели — сетевые паузы не «морозят» картинку.
                    // Пакеты агента возвращают управление, когда подтвердят
                    // позицию (или по страховочному таймауту).
                    let since_move = v.last_move_at.elapsed();
                    let confirmed = v.last_cursor_change_at >= v.last_move_at;
                    let predicting = cursor_predicting(since_move, confirmed);
                    let (nx, ny) = cursor_draw_norm((cs.x, cs.y), v.last_pointer, predicting);
                    let (px, py) = input::norm_to_ui_px(nx, ny, draw.width(), draw.height());
                    let pos = egui::pos2(draw.min.x + px, draw.min.y + py);
                    let remote_h = v
                        .info
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|s| s.out.height as f32)
                        .unwrap_or(720.0)
                        .max(1.0);
                    let scale = (draw.height() / remote_h).clamp(0.2, 4.0);
                    let img = v.cursor_img.lock().unwrap().clone();
                    let ver = v.cursor_img_ver.load(Ordering::Relaxed);
                    if ver != v.cursor_tex_ver {
                        v.cursor_tex_ver = ver;
                        v.cursor_tex = img.as_ref().map(|c| {
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                [c.w as usize, c.h as usize],
                                &c.rgba,
                            );
                            ui.ctx().load_texture(
                                "rat-cursor-img",
                                image,
                                egui::TextureOptions::NEAREST,
                            )
                        });
                    }
                    // Точная картинка — только если она от ТЕКУЩЕГО вида курсора;
                    // иначе рисуем форму (стрелка/палка/рука) — лучше верная форма,
                    // чем чужая картинка.
                    if let (Some(c), Some(tex)) = (&img, v.cursor_tex.as_ref())
                        && c.kind == cs.kind
                    {
                        let min = egui::pos2(
                            pos.x - c.hot_x as f32 * scale,
                            pos.y - c.hot_y as f32 * scale,
                        );
                        let rect = egui::Rect::from_min_size(
                            min,
                            egui::vec2(c.w as f32 * scale, c.h as f32 * scale),
                        );
                        ui.painter().image(
                            tex.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    } else {
                        rat_cursor_ui::paint_cursor(ui.painter(), pos, scale, cs.kind);
                    }
                }
            }

            if back {
                self.close_viewer();
            }
        }
    }

    /// VK печатных клавиш: их доставляет Text-событие, по Key-пути не дублируем.
    fn is_char_vk(vk: u16) -> bool {
        matches!(vk, 0x20 | 0x30..=0x39 | 0x41..=0x5A)
    }

    /// Пора ли рисовать курсор по нашей цели (предсказание, Шаг 1)?
    /// Пока управляем — да; после остановки — пока агент не подтвердит позицию
    /// свежим пакетом (страховочное окно 2 с).
    fn cursor_predicting(since_move: Duration, confirmed: bool) -> bool {
        since_move < Duration::from_millis(300)
            || (!confirmed && since_move < Duration::from_millis(2000))
    }

    /// Норм. координаты для отрисовки удалённого курсора: последняя отправленная
    /// цель (если предсказываем) или пакет агента.
    fn cursor_draw_norm(
        cs: (u16, u16),
        last_pointer: Option<(u16, u16)>,
        predicting: bool,
    ) -> (u16, u16) {
        if predicting {
            last_pointer.unwrap_or(cs)
        } else {
            cs
        }
    }

    /// egui-клавиша → Windows VK (служебные клавиши; печатные идут через vk_for_char).
    fn vk_of_key(key: egui::Key) -> Option<u16> {
        use egui::Key as K;
        Some(match key {
            K::ArrowUp => 0x26,
            K::ArrowDown => 0x28,
            K::ArrowLeft => 0x25,
            K::ArrowRight => 0x27,
            K::Enter => 0x0D,
            K::Escape => 0x1B,
            K::Backspace => 0x08,
            K::Delete => 0x2E,
            K::Insert => 0x2D,
            K::Home => 0x24,
            K::End => 0x23,
            K::PageUp => 0x21,
            K::PageDown => 0x22,
            K::Tab => 0x09,
            K::Space => 0x20,
            K::F1 => 0x70,
            K::F2 => 0x71,
            K::F3 => 0x72,
            K::F4 => 0x73,
            K::F5 => 0x74,
            K::F6 => 0x75,
            K::F7 => 0x76,
            K::F8 => 0x77,
            K::F9 => 0x78,
            K::F10 => 0x79,
            K::F11 => 0x7A,
            K::F12 => 0x7B,
            K::A => 0x41,
            K::B => 0x42,
            K::C => 0x43,
            K::D => 0x44,
            K::E => 0x45,
            K::F => 0x46,
            K::G => 0x47,
            K::H => 0x48,
            K::I => 0x49,
            K::J => 0x4A,
            K::K => 0x4B,
            K::L => 0x4C,
            K::M => 0x4D,
            K::N => 0x4E,
            K::O => 0x4F,
            K::P => 0x50,
            K::Q => 0x51,
            K::R => 0x52,
            K::S => 0x53,
            K::T => 0x54,
            K::U => 0x55,
            K::V => 0x56,
            K::W => 0x57,
            K::X => 0x58,
            K::Y => 0x59,
            K::Z => 0x5A,
            K::Num0 => 0x30,
            K::Num1 => 0x31,
            K::Num2 => 0x32,
            K::Num3 => 0x33,
            K::Num4 => 0x34,
            K::Num5 => 0x35,
            K::Num6 => 0x36,
            K::Num7 => 0x37,
            K::Num8 => 0x38,
            K::Num9 => 0x39,
            _ => return None,
        })
    }

    impl RatApp {
        /// Операция со службой из интерфейса: сразу или через запрос UAC.
        #[cfg(feature = "ffmpeg")]
        fn svc_action(&mut self, op: crate::service::Op) {
            let (flag, name): (&str, &str) = match op {
                crate::service::Op::Install => ("--install-service", "Установка службы"),
                crate::service::Op::Uninstall => ("--uninstall-service", "Удаление службы"),
                crate::service::Op::Start => ("--start-service", "Запуск службы"),
                crate::service::Op::Stop => ("--stop-service", "Остановка службы"),
            };
            // Остановка/удаление из окна: просим службу не завершать помощника —
            // окно не должно пропадать (служба увидит файл-флаг при завершении).
            let keep = matches!(op, crate::service::Op::Stop | crate::service::Op::Uninstall);
            if keep {
                crate::service::raise_keep_helper();
            }
            let result = match op {
                crate::service::Op::Install => crate::service::install(),
                crate::service::Op::Uninstall => crate::service::uninstall(),
                crate::service::Op::Start => crate::service::start(),
                crate::service::Op::Stop => crate::service::stop(),
            };
            if keep {
                crate::service::clear_keep_helper();
            }
            match result {
                Ok(()) => {
                    self.svc_state = crate::service::status();
                    self.svc_checked = Instant::now();
                    match op {
                        crate::service::Op::Install => {
                            self.toast(
                                "Служба установлена — окно закроется, служба сразу подхватит",
                            );
                            self.quit_at = Some(Instant::now() + Duration::from_millis(1500));
                        }
                        crate::service::Op::Uninstall => {
                            self.toast("Служба удалена — приложение продолжает работать");
                        }
                        crate::service::Op::Start => self.toast("Запуск службы: выполнено"),
                        crate::service::Op::Stop => self.toast("Остановка службы: выполнено"),
                    }
                }
                Err(e) => {
                    if e.contains("ошибка 5") {
                        match crate::elevate::relaunch(flag) {
                            Ok(()) => {
                                self.svc_state = crate::service::State::Pending;
                                self.svc_checked = Instant::now();
                                self.toast(format!(
                                    "{name}: подтвердите запрос прав администратора"
                                ));
                            }
                            Err(e2) => self.toast(format!("{name}: {e2}")),
                        }
                    } else {
                        self.toast(format!("{name}: {e}"));
                    }
                }
            }
        }

        /// Отключиться от удалённого ПК и вернуться к списку машин.
        fn close_viewer(&mut self) {
            if let Some(mut s) = self.viewer.take() {
                s.shutdown();
            }
            self.screen = Screen::Home;
        }

        /// Крестик окна прячет приложение в трей, сессии продолжают работать.
        fn handle_close(&mut self, ctx: &egui::Context) {
            // Завершение (выход или smoke-автостоп) — закрытию не мешаем.
            if self.quitting || self.finished {
                return;
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                // Крестик при активной сессии = отключиться и вернуться к
                // списку машин (окно остаётся; повторный крестик — как обычно).
                if self.viewer.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    if ctx.input(|i| i.viewport().fullscreen).unwrap_or(false) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                    }
                    self.close_viewer();
                    return;
                }
                // Крестик в файловом режиме = вернуться к списку машин.
                if self.files.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.close_files();
                    return;
                }
                // Закрытие окна только прячет его: приложение и активные
                // сессии продолжают работать; полный выход — через «Выход»
                // в меню значка (там, где он есть).
                if self.tray.is_none() && !self.args.tray {
                    return;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.hidden = true;
            }
        }

        /// Команды меню значка: показать окно, настройки, выход.
        fn poll_tray(&mut self, ctx: &egui::Context) {
            let Some(cmd) = self.tray.as_ref().and_then(|t| t.poll()) else {
                return;
            };
            match cmd {
                crate::tray::Cmd::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.hidden = false;
                }
                crate::tray::Cmd::Settings => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.hidden = false;
                    self.open_settings();
                }
                crate::tray::Cmd::Quit => {
                    self.quitting = true;
                    self.internet.stop();
                    self.server.stop();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        /// Повторные попытки создать значок: оболочка может быть ещё не готова
        /// (служба и задача стартуют раньше входа пользователя).
        fn retry_tray(&mut self) {
            if self.tray.is_some() || self.tray_retry_left == 0 {
                return;
            }
            if Instant::now() < self.tray_retry_at {
                return;
            }
            // Пока оболочка (проводник) не готова, попытки не расходуем — не
            // создаём значок раньше панели задач (см. комментарий в RatApp::new).
            if !crate::shell_ready() {
                self.tray_retry_at = Instant::now() + Duration::from_secs(2);
                return;
            }
            self.tray_retry_left -= 1;
            self.tray_retry_at = Instant::now() + Duration::from_secs(5);
            let kind = if self.args.service_helper && !self.svc_detached {
                crate::tray::Kind::Service
            } else {
                crate::tray::Kind::Manual
            };
            match crate::tray::Tray::new(kind) {
                Ok(tray) => {
                    crate::note("[ui] значок в трее создан (повторная попытка)");
                    self.tray = Some(tray);
                    self.tray_retry_left = 0;
                }
                Err(e) => {
                    crate::note(&format!("[ui] значок в трее не создан: {e}"));
                    // Неудачная попытка могла оставить «пустое» служебное окно
                    // значка — уберём его, чтобы при перезапуске проводника оно
                    // не «оживилось» вторым значком.
                    crate::tray::cleanup_orphans();
                    if self.tray_retry_left == 0 {
                        crate::note("[ui] значок в трее: попытки исчерпаны");
                    }
                }
            }
        }

        /// Помощник службы: если служба остановлена/удалена, продолжаем работу
        /// как обычное приложение — окно не должно пропадать.
        fn check_service_detach(&mut self) {
            if !self.args.service_helper || self.svc_detached {
                return;
            }
            if self.svc_probe_checked.elapsed() < Duration::from_secs(2) {
                return;
            }
            self.svc_probe_checked = Instant::now();
            if !matches!(
                crate::service::status(),
                crate::service::State::NotInstalled | crate::service::State::Stopped
            ) {
                return;
            }
            self.svc_detached = true;
            crate::note("[ui] служба остановлена — продолжаю работать как обычное приложение");
            match crate::tray::Tray::new(crate::tray::Kind::Manual) {
                Ok(tray) => self.tray = Some(tray),
                Err(e) => crate::note(&format!("[ui] значок трея не заменён: {e}")),
            }
            self.toast("Служба остановлена — приложение продолжает работать");
        }
    }

    impl eframe::App for RatApp {
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            let ctx = ui.ctx().clone();
            // Первый кадр: запомнить контекст egui — поток слушателя
            // повторного запуска будит им перерисовку.
            if let Ok(mut slot) = self.repaint_slot.lock()
                && slot.is_none()
            {
                *slot = Some(ctx.clone());
            }
            // Повторный запуск: показать окно (как клик по значку в трее).
            if self.show_request.as_ref().is_some_and(|s| s.take()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                self.hidden = false;
            }
            // Запрос вежливого завершения (установщик/обновление): выходим
            // штатно, чтобы значок трея убрался сам.
            if self.quit_request.as_ref().is_some_and(|q| q.take()) {
                crate::note("[ui] получен запрос завершения — выхожу");
                self.quitting = true;
                self.internet.stop();
                self.server.stop();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            self.poll_tray(&ctx);
            self.handle_close(&ctx);
            self.retry_tray();
            self.check_service_detach();
            if let Some(t) = self.quit_at
                && Instant::now() >= t
            {
                self.quit_at = None;
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if !self.tray_hide_done {
                self.tray_hide_done = true;
                // Режим агента (--tray, служба): окно стартует скрытым даже без
                // значка трея — на управляемой машине окно не должно висеть.
                if self.args.tray {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    self.hidden = true;
                }
            }
            if !self.autoconnect_done {
                self.autoconnect_done = true;
                if let Some(raw_addr) = self.args.autoconnect.clone() {
                    let addr = match self.device.mode {
                        ConnectionMode::Internet => raw_addr.trim().to_string(),
                        ConnectionMode::Lan => normalize_addr(&raw_addr),
                    };
                    if self.device.mode == ConnectionMode::Internet && !is_valid_id(&addr) {
                        self.toast("Для интернет-подключения нужен ID из 6 цифр");
                    } else {
                        self.start_connect(addr.clone(), addr);
                    }
                }
            }
            if !self.files_ui_done
                && let Some(addr) = self.args.files_ui.clone()
            {
                self.files_ui_done = true;
                let target = rat_session::files_worker::Target::Tcp(addr.clone());
                self.files = Some(FilesState::open("Тест".to_string(), addr.clone(), target));
                self.screen = Screen::Files;
            }
            self.poll_connect();
            match self.screen {
                Screen::Home => self.home_ui(ui),
                Screen::Viewer => self.viewer_ui(ui),
                Screen::Files => self.files_ui(ui),
            }
            self.dialogs(ui.ctx());
            if let Some(secs) = self.args.smoke
                && self.started.elapsed().as_secs_f32() >= secs
                && !self.finished
            {
                self.finished = true;
                if let Some(v) = self.viewer.as_mut() {
                    v.shutdown();
                }
                self.server.stop();
                crate::note("APP_DONE");
            }
            if self.finished {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if self.hidden {
                // Окно в трее: сессии продолжают работать, UI просыпается редко.
                ctx.request_repaint_after(Duration::from_millis(250));
            } else {
                ctx.request_repaint_after(Duration::from_millis(10));
            }
        }
    }

    #[cfg(test)]
    mod files_util_tests {
        use super::*;

        #[test]
        fn sel_click_behaviour() {
            let mut sel = BTreeSet::new();
            let mut anchor = None;
            sel_click(&mut sel, &mut anchor, 3, 10, false, false);
            assert_eq!(sel.iter().copied().collect::<Vec<_>>(), vec![3]);
            sel_click(&mut sel, &mut anchor, 5, 10, true, false);
            assert_eq!(sel.iter().copied().collect::<Vec<_>>(), vec![3, 5]);
            sel_click(&mut sel, &mut anchor, 3, 10, true, false);
            assert_eq!(sel.iter().copied().collect::<Vec<_>>(), vec![5]);
            sel_click(&mut sel, &mut anchor, 2, 10, false, false);
            sel_click(&mut sel, &mut anchor, 7, 10, false, true);
            assert_eq!(
                sel.iter().copied().collect::<Vec<_>>(),
                (2..=7).collect::<Vec<_>>()
            );
            sel_click(&mut sel, &mut anchor, 0, 10, false, false);
            assert_eq!(sel.iter().copied().collect::<Vec<_>>(), vec![0]);
        }

        #[test]
        fn crumbs_split_paths() {
            let c = crumbs("C:\\Users\\ad\\Documents");
            let labels: Vec<&str> = c.iter().map(|(l, _)| l.as_str()).collect();
            assert_eq!(labels, vec!["C:", "Users", "ad", "Documents"]);
            assert_eq!(c[0].1, "C:\\");
            assert_eq!(c[3].1, "C:\\Users\\ad\\Documents");
            let r = crumbs("D:\\1c_base");
            assert_eq!(r.len(), 2);
            assert_eq!(r[1].1, "D:\\1c_base");
            assert_eq!(crumbs("C:\\").len(), 1);
        }

        #[test]
        fn plural_objects() {
            assert_eq!(obj_word(1), "объект");
            assert_eq!(obj_word(3), "объекта");
            assert_eq!(obj_word(7), "объектов");
            assert_eq!(obj_word(11), "объектов");
            assert_eq!(obj_word(21), "объект");
        }

        #[test]
        fn icon_colors_by_type() {
            assert_eq!(icon_color("a.XLSX"), egui::Color32::from_rgb(86, 166, 100));
            assert_eq!(icon_color("b.zip"), egui::Color32::from_rgb(155, 122, 210));
            assert_eq!(
                icon_color("без_точки"),
                egui::Color32::from_rgb(155, 164, 175)
            );
        }

        #[test]
        fn human_size_comma() {
            assert_eq!(rat_session::files::human_size(1536), "1,5 КБ");
            assert_eq!(rat_session::files::human_size(500), "500 Б");
        }

        #[test]
        fn cursor_prediction_windows() {
            let ms = Duration::from_millis;
            assert!(cursor_predicting(ms(100), false));
            assert!(cursor_predicting(ms(500), false));
            assert!(!cursor_predicting(ms(500), true));
            assert!(!cursor_predicting(ms(2500), false));
            assert!(!cursor_predicting(ms(2500), true));
            assert_eq!(cursor_draw_norm((10, 20), Some((30, 40)), true), (30, 40));
            assert_eq!(cursor_draw_norm((10, 20), Some((30, 40)), false), (10, 20));
            assert_eq!(cursor_draw_norm((10, 20), None, true), (10, 20));
        }
    }
}
