//! `rat-relay` — relay-сервер Rat Next: pairing по ID и форвард непрозрачных
//! байтов сессии. Обычный запуск — окно с журналом и иконкой в трее.
//!
//! Запуск:
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

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod autostart;
mod core;
mod elevate;
mod icon;
mod port;
mod service;
mod settings;
mod tray;
mod ui;

use std::sync::atomic::Ordering;
use std::time::Duration;

fn main() {
    let mut listen: Option<String> = None;
    let mut smoke: Option<f32> = None;

    let mut remove_autostart = false;
    let mut start_hidden = false;
    let mut install_service = false;
    let mut uninstall_service = false;
    let mut start_service = false;
    let mut stop_service = false;
    let mut service_helper = false;
    let mut service_mode = false;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--listen" if i + 1 < args.len() => {
                listen = Some(args[i + 1].clone());
                i += 2;
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
