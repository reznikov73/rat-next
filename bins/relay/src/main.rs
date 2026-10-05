//! `rat-relay` — relay-сервер Rat Next: pairing по ID и форвард непрозрачных
//! байтов сессии. Обычный запуск — окно с журналом и иконкой в трее.
//!
//! Запуск:
//! ```text
//! rat-relay.exe                          # GUI, адрес из relay.json (по умолчанию 0.0.0.0:8889)
//! rat-relay.exe --listen 0.0.0.0:8889    # GUI с указанным адресом прослушивания
//! rat-relay.exe --smoke=10               # без GUI: 10 секунд и выход (проверки)
//! ```

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod core;
mod port;
mod settings;
mod tray;
mod ui;

use std::sync::atomic::Ordering;
use std::time::Duration;

fn main() {
    let mut listen: Option<String> = None;
    let mut smoke: Option<f32> = None;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--listen" if i + 1 < args.len() => {
                listen = Some(args[i + 1].clone());
                i += 2;
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
    if let Some(secs) = smoke {
        run_headless(listen, secs);
        return;
    }
    if let Err(e) = ui::run(listen) {
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
