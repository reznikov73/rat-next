//! `rat-relay` — relay-сервер Rat Next: pairing по ID и форвард непрозрачных
//! байтов сессии.
//!
//! Запуск (Windows):
//! ```text
//! rat-relay.exe                          # GUI, адрес из relay.json (по умолчанию 0.0.0.0:8889)
//! rat-relay.exe --listen 0.0.0.0:8889    # GUI с указанным адресом прослушивания
//! rat-relay.exe --smoke=10               # без GUI: 10 секунд и выход (проверки)
//! rat-relay.exe --install-autostart       # включить автозапуск (задача «Rat Next relay», старт в трей)
//! rat-relay.exe --remove-autostart        # выключить автозапуск
//! rat-relay.exe --tray                    # старт сразу свёрнутым в трей
//! rat-relay.exe --install-service         # поставить службу (автозапуск при загрузке Windows)
//! rat-relay.exe --uninstall-service       # снять службу
//! rat-relay.exe --start-service           # запустить службу (использует интерфейс)
//! rat-relay.exe --stop-service            # остановить службу (использует интерфейс)
//! rat-relay.exe --service-helper          # хелпер службы (запускает сама служба)
//! ```
//!
//! Запуск (Linux):
//! ```text
//! rat-relay --listen 0.0.0.0:8889   # сервер на переднем плане (для systemd)
//! rat-relay --install-service       # поставить unit rat-relay.service и запустить
//! rat-relay --uninstall-service     # остановить и снять unit
//! rat-relay --start-service         # systemctl start
//! rat-relay --stop-service          # systemctl stop
//! rat-relay --smoke=10              # проверка: поднять порт на 10 секунд и выйти
//! rat-relay --install-autostart     # автозапуск окна при входе (XDG autostart)
//! ```

// Атрибут применим только к Windows: на Linux `windows_subsystem` неизвестен.
// Скрываем консоль только в GUI-сборке: в headless-режиме stdout — это
// единственный канал журнала (перенаправляется в файл или journald).
#![cfg_attr(
    all(windows, feature = "gui", not(test)),
    windows_subsystem = "windows"
)]
// В headless-сборке (`--no-default-features`) часть API остаётся ради
// совместимости с GUI-режимом и Windows-версией: `port` (диагностика порта
// в окне), `elevate` (кнопки сервиса в окне), поля `core::Event` (адресаты
// событий в журнале окна). Без этого разрешения они дают `dead_code`, а CI
// собирает с `-D warnings`. Ослабляем только в headless-профиле.
#![cfg_attr(not(feature = "gui"), allow(dead_code))]

// Модули с платформенными реализациями. Имена совпадают, поэтому остальной
// код (CLI в `main`, окно в `ui`) не знает, под какой ОС собран.
#[cfg(windows)]
mod autostart;
#[cfg(target_os = "linux")]
#[path = "autostart_linux.rs"]
mod autostart;

mod core;

#[cfg(windows)]
mod elevate;
#[cfg(target_os = "linux")]
#[path = "elevate_linux.rs"]
mod elevate;

mod icon;

#[cfg(windows)]
mod port;
#[cfg(target_os = "linux")]
#[path = "port_linux.rs"]
mod port;

#[cfg(windows)]
mod service;
#[cfg(target_os = "linux")]
#[path = "service_linux.rs"]
mod service;

mod settings;

// Окно и трей — только при включённой фиче `gui` (по умолчанию включена;
// на сервере без графической сессии собирают `--no-default-features`).
#[cfg(feature = "gui")]
mod tray;
#[cfg(feature = "gui")]
mod ui;

use std::sync::atomic::Ordering;
use std::time::Duration;

fn main() {
    let mut listen: Option<String> = None;
    let mut smoke: Option<f32> = None;

    let mut remove_autostart = false;
    let mut install_autostart = false;
    let mut start_hidden = false;
    let mut install_service = false;
    let mut uninstall_service = false;
    let mut start_service = false;
    let mut stop_service = false;
    let mut service_helper = false;
    let mut service_mode = false;
    let mut request_quit = false;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--listen" if i + 1 < args.len() => {
                listen = Some(args[i + 1].clone());
                i += 2;
            }

            "--install-autostart" => {
                install_autostart = true;
                i += 1;
            }
            "--remove-autostart" => {
                remove_autostart = true;
                i += 1;
            }
            "--tray" => {
                start_hidden = true;
                i += 1;
            }
            "--install-service" => {
                install_service = true;
                i += 1;
            }
            "--uninstall-service" => {
                uninstall_service = true;
                i += 1;
            }
            "--start-service" => {
                start_service = true;
                i += 1;
            }
            "--stop-service" => {
                stop_service = true;
                i += 1;
            }
            "--service-helper" => {
                service_helper = true;
                i += 1;
            }
            "--service" => {
                service_mode = true;
                i += 1;
            }
            "--request-quit" => {
                request_quit = true;
                i += 1;
            }
            s if s.starts_with("--smoke=") => {
                smoke = s["--smoke=".len()..].parse().ok();
                i += 1;
            }
            other => {
                eprintln!("неизвестный аргумент: {other}");
                i += 1;
            }
        }
    }
    // Вежливое завершение работающего окна relay (установщик, скрипты):
    // доставляем сигнал и выходим, ничего не запуская.
    if request_quit {
        let delivered = rat_session::single_instance::quit_signal("relay");
        println!(
            "relay: запрос завершения {}",
            if delivered {
                "отправлен"
            } else {
                "не доставлен"
            }
        );
        std::process::exit(0);
    }
    // Задача-автозапуск «Rat Next relay» больше не используется — убираем
    // устаревшую, если она осталась с прошлых версий.
    if autostart::is_installed() {
        let _ = autostart::remove();
    }
    if install_service || uninstall_service {
        let res = if install_service {
            service::install()
        } else {
            service::uninstall()
        };
        match res {
            Ok(()) => {
                println!("служба relay обновлена");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("service: {e}");
                std::process::exit(1);
            }
        }
    }
    if start_service {
        match service::start() {
            Ok(()) => {
                println!("Служба relay запущена.");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("service: {e}");
                std::process::exit(1);
            }
        }
    }
    if stop_service {
        match service::stop() {
            Ok(()) => {
                println!("Служба relay остановлена.");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("service: {e}");
                std::process::exit(1);
            }
        }
    }
    if service_mode {
        // У службы отдельный экземплярный мьютекс: GUI-версия и служба не считаются
        // «двойным запуском» (конфликт порта, если что, виден в интерфейсе).
        if !rat_session::single_instance::try_acquire("relay-svc") {
            std::process::exit(2);
        }
        if let Err(e) = service::run_service() {
            eprintln!("service: {e}");
            std::process::exit(1);
        }
        return;
    }
    // Хелпер службы существует только в Windows-модели: SCM запускает GUI
    // отдельно от службы в пользовательской сессии. На Linux окно (если оно
    // собрано) запускает сам пользователь, а сервером управляет systemd.
    #[cfg(all(windows, feature = "gui"))]
    if service_helper {
        // Хелпер службы: GUI-процесс в пользовательской сессии. Собственный
        // цикл relay не поднимает (порт держит служба): показывает окно и
        // значок; повторный запуск rat-relay.exe открывает это окно.
        if !rat_session::single_instance::try_acquire("relay") {
            // Не выходим: ждём, пока закроется другое окно (как у rat);
            // служба остаётся спокойной и сразу подхватывает управление.
            let mut steps: u32 = 0;
            loop {
                if rat_session::single_instance::try_acquire("relay") {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
                steps += 1;
                if steps.is_multiple_of(20)
                    && matches!(
                        service::status(),
                        service::State::NotInstalled | service::State::Stopped
                    )
                {
                    std::process::exit(0);
                }
            }
        }
        if let Err(e) = ui::run(listen, true, ui::UiMode::Helper) {
            eprintln!("relay: {e}");
            std::process::exit(1);
        }
        return;
    }

    #[cfg(not(all(windows, feature = "gui")))]
    if service_helper {
        eprintln!(
            "relay: флаг --service-helper требует GUI-сборки Windows.\n\
             На Linux сервисом управляет systemd: используйте `--install-service`."
        );
        std::process::exit(2);
    }

    if install_autostart {
        match autostart::install() {
            Ok(()) => {
                println!("автозапуск relay включён.");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("autostart: {e}");
                std::process::exit(1);
            }
        }
    }
    if remove_autostart {
        match autostart::remove() {
            Ok(()) => {
                println!("автозапуск relay удалён.");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("autostart: {e}");
                std::process::exit(1);
            }
        }
    }
    if !rat_session::single_instance::try_acquire("relay") {
        if smoke.is_none() {
            // Повторный запуск — просим работающий экземпляр показать окно
            // (то же, что клик по значку в трее; работает и когда значка нет).
            if rat_session::single_instance::show_signal("relay") {
                std::process::exit(0);
            }
            rat_session::single_instance::warn(
                "Rat Next relay",
                "Relay уже запущен.\nОкно можно открыть из значка в трее.",
            );
        }
        eprintln!("relay: экземпляр уже запущен");
        std::process::exit(2);
    }

    if let Some(secs) = smoke {
        run_headless(listen, secs);
        return;
    }

    // Дальше — либо окно, либо (без фичи `gui`) работа на переднем плане.
    #[cfg(feature = "gui")]
    {
        let mode = if service::status() == service::State::NotInstalled {
            ui::UiMode::Standalone
        } else {
            ui::UiMode::Attach
        };
        if let Err(e) = ui::run(listen, start_hidden, mode) {
            eprintln!("relay: {e}");
            std::process::exit(1);
        }
    }

    #[cfg(not(feature = "gui"))]
    {
        let _ = start_hidden;
        run_foreground(listen);
    }
}

/// Работа без окна: сервер держит порт до получения сигнала завершения.
///
/// Это основной режим для systemd (и для машины без графической сессии).
/// Флаг `--smoke=N` использует тот же код с таймером.
#[cfg(not(feature = "gui"))]
fn run_foreground(listen: Option<String>) {
    let configured = settings::load();
    let addr = listen.unwrap_or(configured.listen_addr);
    let handle = match core::Handle::start(&addr) {
        Ok(handle) => handle,
        Err(e) => {
            eprintln!("relay: {e}");
            std::process::exit(1);
        }
    };
    println!("RELAY_LISTENING {addr}");

    // Устанавливаем обработчик сигналов (SIGTERM от systemd, SIGINT с Ctrl+C).
    install_shutdown_handler();

    // Журнал событий ядра — в stdout (systemd перенаправит в journald).
    //
    // `recv_timeout` вместо блокирующего `recv` принципиален: сигнал может
    // быть доставлен любому потоку процесса, в том числе тому, который висит
    // в приёме событий, а `std` после EINTR прозрачно повторяет вызов —
    // блокирующий `recv` остановку бы не заметил (проверено на практике:
    // SIGTERM не завершал процесс).
    while !SHUTDOWN.load(Ordering::SeqCst) {
        match handle.events.recv_timeout(Duration::from_millis(100)) {
            Ok(core::Event::Log(line)) => println!("{line}"),
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Просим ядро остановить приём и дожидаемся потока листенера.
    handle.request_stop();
    handle.join();
    println!("RELAY_STOPPED");
}

/// Флаг «получен сигнал завершения» — общий для всего процесса.
#[cfg(not(feature = "gui"))]
static SHUTDOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Обработчик сигнала: только взводит флаг (асинхронно-безопасно).
#[cfg(not(feature = "gui"))]
extern "C" fn on_shutdown_signal(_sig: i32) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

/// Зарегистрировать SIGTERM и SIGINT.
///
/// Внешние крейты (`signal-hook`, `libc`) не нужны: достаточно двух
/// объявлений `extern "C"` — сигнатуры совпадают с POSIX на Linux.
#[cfg(not(feature = "gui"))]
fn install_shutdown_handler() {
    unsafe extern "C" {
        fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    }

    const SIGINT: i32 = 2;
    const SIGTERM: i32 = 15;

    // SAFETY: регистрируем корректный extern "C"-обработчик; он делает только
    // атомарную запись, что допустимо в обработчике сигнала.
    unsafe {
        signal(SIGINT, on_shutdown_signal);
        signal(SIGTERM, on_shutdown_signal);
    }
}

/// Проверочный режим без GUI: `RELAY_LISTENING <адрес>`, работа `secs` секунд, выход.
fn run_headless(listen: Option<String>, secs: f32) {
    let configured = settings::load();
    let addr = listen.unwrap_or(configured.listen_addr);
    let handle = match core::Handle::start(&addr) {
        Ok(handle) => handle,
        Err(e) => {
            eprintln!("relay: {e}");
            std::process::exit(1);
        }
    };
    println!("RELAY_LISTENING {addr}");
    let stop = handle.stop.clone();
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs_f32(secs));
        stop.store(true, Ordering::Relaxed);
    });
    while let Ok(event) = handle.events.recv() {
        if let core::Event::Log(line) = event {
            println!("{line}");
        }
    }
    handle.join();
    let _ = timer.join();
    println!("RELAY_STOPPED");
}
