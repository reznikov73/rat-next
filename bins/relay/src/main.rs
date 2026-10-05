//! `rat-relay` — relay-сервер Rat Next: pairing вьюера и агента по ID
//! и форвард непрозрачных байтов сессии (содержимое не читается).
//!
//! Протокол (построчно, `\n`):
//! - агент:  `REG <id> <name>` → `OK`, затем ждёт `PAIRED` → сырой режим;
//! - вьюер:  `CONNECT <id>`    → `OK` (агенту уходит `PAIRED`) или `ERR`, затем сырой режим;
//! - статус: `WHO <id>`        → `OK`/`ERR`, соединение закрывается.
//!
//! Запуск: `rat-relay.exe [--listen 0.0.0.0:55141] [--smoke=N]`

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Registry = Arc<Mutex<HashMap<String, (u64, mpsc::Sender<TcpStream>)>>>;

static NEXT_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn main() {
    let mut listen = "0.0.0.0:55141".to_string();
    let mut smoke: Option<f32> = None;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--listen" if i + 1 < args.len() => {
                listen = args[i + 1].clone();
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
    let listener = match TcpListener::bind(&listen) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind {listen}: {e}");
            std::process::exit(1);
        }
    };
    println!("RELAY_LISTENING {listen}");
    let stop = Arc::new(AtomicBool::new(false));
    if let Some(secs) = smoke {
        let s2 = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f32(secs));
            s2.store(true, Ordering::Relaxed);
        });
    }
    serve(listener, stop);
    println!("RELAY_STOPPED");
}

/// Цикл приёма подключений (используется main и тестами).
fn serve(listener: TcpListener, stop: Arc<AtomicBool>) {
    if listener.set_nonblocking(true).is_err() {
        println!("[relay] set_nonblocking не удался");
        return;
    }
    let registry: Registry = Arc::new(Mutex::new(HashMap::new()));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, peer)) => {
                let registry = registry.clone();
                std::thread::spawn(move || {
                    // Явно блокирующий режим: не наследуем nonblocking от листенера.
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_nodelay(true);
                    if let Err(e) = handle_conn(stream, &registry) {
                        println!("[relay] {peer}: {e}");
                    }
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                println!("[relay] accept: {e}");
                std::thread::sleep(Duration::from_millis(300));
            }
        }
    }
}

fn handle_conn(mut stream: TcpStream, registry: &Registry) -> io::Result<()> {
    let line = read_line(&mut stream)?;
    let mut parts = line.splitn(3, ' ');
    let cmd = parts.next().unwrap_or("");
    match cmd {
        "WHO" => {
            let id = parts.next().unwrap_or("");
            let online = registry.lock().unwrap().contains_key(id);
            send_line(&mut stream, if online { "OK" } else { "ERR OFFLINE" })?;
            let _ = stream.shutdown(Shutdown::Both);
        }
        "REG" => {
            let id = parts.next().unwrap_or("").to_string();
            let name = parts.next().unwrap_or("").to_string();
            if id.is_empty() {
                send_line(&mut stream, "ERR BADID")?;
                return Ok(());
            }
            let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
            let (tx, rx) = mpsc::channel::<TcpStream>();
            registry.lock().unwrap().insert(id.clone(), (token, tx));
            send_line(&mut stream, "OK")?;
            println!("[relay] регистрация: {id} ({name})");
            let viewer = match rx.recv() {
                Ok(v) => v,
                Err(_) => return Ok(()),
            };
            {
                let mut reg = registry.lock().unwrap();
                if reg.get(&id).map(|(t, _)| *t == token).unwrap_or(false) {
                    reg.remove(&id);
                }
            }
            send_line(&mut stream, "PAIRED")?;
            println!("[relay] {id}: сессия началась");
            pump(stream, viewer);
            println!("[relay] {id}: сессия завершена");
        }
        "CONNECT" => {
            let id = parts.next().unwrap_or("");
            let slot = registry.lock().unwrap().remove(id);
            match slot {
                Some((_, tx)) => {
                    send_line(&mut stream, "OK")?;
                    println!("[relay] {id}: вьюер найден");
                    let _ = tx.send(stream);
                }
                None => {
                    send_line(&mut stream, "ERR OFFLINE")?;
                    let _ = stream.shutdown(Shutdown::Both);
                }
            }
        }
        _ => {
            send_line(&mut stream, "ERR UNKNOWN")?;
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
    Ok(())
}

/// Форвард байтов между сокетами в обе стороны до закрытия любого из них.
fn pump(a: TcpStream, b: TcpStream) {
    let (mut a_r, mut a_w) = match (a.try_clone(), a.try_clone()) {
        (Ok(r), Ok(w)) => (r, w),
        _ => return,
    };
    let (mut b_r, mut b_w) = match (b.try_clone(), b.try_clone()) {
        (Ok(r), Ok(w)) => (r, w),
        _ => return,
    };
    let t1 = std::thread::spawn(move || {
        let _ = io::copy(&mut a_r, &mut b_w);
        let _ = b_w.shutdown(Shutdown::Both);
    });
    let t2 = std::thread::spawn(move || {
        let _ = io::copy(&mut b_r, &mut a_w);
        let _ = a_w.shutdown(Shutdown::Both);
    });
    let _ = t1.join();
    let _ = t2.join();
}

fn read_line(stream: &mut TcpStream) -> io::Result<String> {
    let mut out = Vec::with_capacity(32);
    let mut b = [0u8; 1];
    loop {
        let n = stream.read(&mut b)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "соединение закрыто",
            ));
        }
        if b[0] == b'\n' {
            break;
        }
        out.push(b[0]);
        if out.len() > 256 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "строка слишком длинная",
            ));
        }
    }
    String::from_utf8(out).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "не UTF-8"))
}

fn send_line(stream: &mut TcpStream, line: &str) -> io::Result<()> {
    let mut s = String::with_capacity(line.len() + 1);
    s.push_str(line);
    s.push('\n');
    stream.write_all(s.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start_relay() -> (
        std::net::SocketAddr,
        Arc<AtomicBool>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let h = std::thread::spawn(move || serve(listener, stop2));
        (addr, stop, h)
    }

    #[test]
    fn pair_pumps_bytes() {
        let (addr, stop, srv) = start_relay();
        let (stream, _abort) = rat_session::link::relay_register(addr, "T-1", "тест").unwrap();
        let agent = std::thread::spawn(move || {
            let (link, _cl) = rat_session::link::relay_wait_paired(stream).unwrap();
            link.tx.send(b"from-agent".to_vec()).unwrap();
            let got = link.rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(got, b"from-viewer".to_vec());
        });
        let (link, _cl) = rat_session::link::relay_connect(addr, "T-1").unwrap();
        let got = link.rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(got, b"from-agent".to_vec());
        link.tx.send(b"from-viewer".to_vec()).unwrap();
        agent.join().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(!rat_session::link::relay_who(addr, "T-1").unwrap());
        stop.store(true, Ordering::Relaxed);
        let _ = srv.join();
    }

    #[test]
    fn offline_connect_errors() {
        let (addr, stop, srv) = start_relay();
        assert!(rat_session::link::relay_connect(addr, "NOBODY").is_err());
        assert!(!rat_session::link::relay_who(addr, "NOBODY").unwrap());
        stop.store(true, Ordering::Relaxed);
        let _ = srv.join();
    }
}
