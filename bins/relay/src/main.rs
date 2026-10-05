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
//! ```

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod autostart;
mod core;
mod icon;
mod port;
mod settings;
mod tray;
mod ui;

use std::sync::atomic::Ordering;
use std::time::Duration;

fn main() {
    let mut listen: Option<String> = None;
    let mut smoke: Option<f32> = None;
    let mut install_autostart = false;
    let mut remove_autostart = false;
    let mut start_hidden = false;
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
    if install_autostart || remove_autostart {
        let res = if install_autostart {
            autostart::install()
        } else {
            autostart::remove()
        };
        match res {
            Ok(()) => {
                println!("автозапуск relay обновлён");
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
    if let Err(e) = ui::run(listen, start_hidden) {
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
