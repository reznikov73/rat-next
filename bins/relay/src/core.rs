//! Ядро relay-сервера Rat Next: pairing вьюера и агента по ID
//! и форвард непрозрачных байтов сессии (содержимое не читается).
//!
//! Протокол (построчно, `\n`):
//! - агент:  `REG <id> <name>` → `OK`, затем ждёт `PAIRED` → сырой режим;
//! - вьюер:  `CONNECT <id>`    → `OK` (агенту уходит `PAIRED`) или `ERR`, затем сырой режим;
//! - статус: `WHO <id>`        → `OK`/`ERR`, соединение закрывается.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Событие ядра для UI и журнала.
#[derive(Debug, Clone)]
pub enum Event {
    Log(String),
    Registered { id: String, name: String },
    SessionStart { id: String },
    SessionEnd { id: String },
}

/// Канал событий: ядро не знает, кто их читает (окно, консоль, тесты).
pub type Sink = Sender<Event>;

pub fn log(sink: &Sink, message: impl Into<String>) {
    let _ = sink.send(Event::Log(message.into()));
}

type Registry = Arc<Mutex<HashMap<String, (u64, Sender<TcpStream>)>>>;

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Цикл приёма подключений. Выходит, когда `stop` выставлен в `true`.
pub fn serve(listener: TcpListener, stop: Arc<AtomicBool>, sink: Sink) {
    if listener.set_nonblocking(true).is_err() {
        log(&sink, "[relay] set_nonblocking не удался");
        return;
    }
    let registry: Registry = Arc::new(Mutex::new(HashMap::new()));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, peer)) => {
                let registry = registry.clone();
                let sink = sink.clone();
                std::thread::spawn(move || {
                    // Явно блокирующий режим: не наследуем nonblocking от листенера.
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_nodelay(true);
                    if let Err(e) = handle_conn(stream, &registry, &sink) {
                        log(&sink, format!("[relay] {peer}: {e}"));
                    }
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                log(&sink, format!("[relay] accept: {e}"));
                std::thread::sleep(Duration::from_millis(300));
            }
        }
    }
    log(&sink, "[relay] сервер остановлен");
}

fn handle_conn(mut stream: TcpStream, registry: &Registry, sink: &Sink) -> io::Result<()> {
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
            let (tx, rx) = std::sync::mpsc::channel::<TcpStream>();
            registry.lock().unwrap().insert(id.clone(), (token, tx));
            send_line(&mut stream, "OK")?;
            log(sink, format!("[relay] регистрация: {id} ({name})"));
            let _ = sink.send(Event::Registered {
                id: id.clone(),
                name: name.clone(),
            });
            let viewer = match rx.recv() {
                Ok(v) => v,
                Err(_) => {
                    let mut reg = registry.lock().unwrap();
                    if reg.get(&id).map(|(t, _)| *t == token).unwrap_or(false) {
                        reg.remove(&id);
                    }
                    return Ok(());
                }
            };
            {
                let mut reg = registry.lock().unwrap();
                if reg.get(&id).map(|(t, _)| *t == token).unwrap_or(false) {
                    reg.remove(&id);
                }
            }
            send_line(&mut stream, "PAIRED")?;
            log(sink, format!("[relay] {id}: сессия началась"));
            let _ = sink.send(Event::SessionStart { id: id.clone() });
            pump(stream, viewer);
            log(sink, format!("[relay] {id}: сессия завершена"));
            let _ = sink.send(Event::SessionEnd { id });
        }
        "CONNECT" => {
            let id = parts.next().unwrap_or("");
            let slot = registry.lock().unwrap().remove(id);
            match slot {
                Some((_, tx)) => {
                    send_line(&mut stream, "OK")?;
                    log(sink, format!("[relay] {id}: вьюер найден"));
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

/// Запустить relay в фоновом потоке и вернуть хэндл управления.
pub struct Handle {
    pub stop: Arc<AtomicBool>,
    pub events: Receiver<Event>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Handle {
    /// Привязывает порт и стартует приём подключений.
    pub fn start(listen: &str) -> Result<Self, String> {
        let listener = TcpListener::bind(listen).map_err(|e| format!("bind {listen}: {e}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let stop2 = stop.clone();
        let join = std::thread::spawn(move || serve(listener, stop2, tx));
        Ok(Self {
            stop,
            events: rx,
            join: Some(join),
        })
    }

    /// Сигнал остановки. Порт освобождается, когда поток приёма завершится.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.join.as_ref().map(|h| h.is_finished()).unwrap_or(true)
    }

    /// Дождаться завершения потока приёма (не для UI-потока).
    pub fn join(mut self) {
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Relay = (
        std::net::SocketAddr,
        Arc<AtomicBool>,
        Receiver<Event>,
        std::thread::JoinHandle<()>,
    );

    fn start_relay() -> Relay {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let h = std::thread::spawn(move || serve(listener, stop2, tx));
        (addr, stop, rx, h)
    }

    fn drain(rx: &Receiver<Event>, want: fn(&Event) -> bool) -> bool {
        while let Ok(e) = rx.recv_timeout(Duration::from_secs(2)) {
            if want(&e) {
                return true;
            }
        }
        false
    }

    #[test]
    fn pair_pumps_bytes() {
        let (addr, stop, rx, srv) = start_relay();
        let (stream, _abort) = rat_session::link::relay_register(addr, "T-1", "тест").unwrap();
        assert!(drain(&rx, |e| matches!(e, Event::Registered { .. })));
        let agent = std::thread::spawn(move || {
            let (link, _cl) = rat_session::link::relay_wait_paired(stream).unwrap();
            link.tx.send(b"from-agent".to_vec()).unwrap();
            let got = link.rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(got, b"from-viewer".to_vec());
        });
        let (link, _cl) = rat_session::link::relay_connect(addr, "T-1").unwrap();
        assert!(drain(&rx, |e| matches!(e, Event::SessionStart { .. })));
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
        let (addr, stop, _rx, srv) = start_relay();
        assert!(rat_session::link::relay_connect(addr, "NOBODY").is_err());
        assert!(!rat_session::link::relay_who(addr, "NOBODY").unwrap());
        stop.store(true, Ordering::Relaxed);
        let _ = srv.join();
    }

    #[test]
    fn handle_reports_stop() {
        let handle = Handle::start("127.0.0.1:0").unwrap();
        assert!(!handle.is_finished());
        handle.request_stop();
        std::thread::sleep(Duration::from_millis(400));
        assert!(handle.is_finished());
    }
}
