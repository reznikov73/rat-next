//! Линк сессии: байтовый дуплекс между сторонами.
//!
//! Два варианта: in-memory (`duplex_pair`, для тестов) и TCP (`tcp_connect` /
//! `tcp_accept`, первый сетевой транспорт пилота). Mux транспорт-агностичен —
//! позже сюда встанет QUIC.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

/// Дуплекс: `tx` — отправка байтов «в сеть», `rx` — приём байтов «из сети».
pub struct Duplex {
    pub tx: Sender<Vec<u8>>,
    pub rx: Receiver<Vec<u8>>,
}

/// Создать пару связанных концов duplex (для тестов и loopback).
pub fn duplex_pair() -> (Duplex, Duplex) {
    let (a_tx, b_rx) = channel::<Vec<u8>>();
    let (b_tx, a_rx) = channel::<Vec<u8>>();
    (Duplex { tx: a_tx, rx: a_rx }, Duplex { tx: b_tx, rx: b_rx })
}

/// Дескриптор закрытия TCP-линка (shutdown сокета будит потоки чтения/записи).
pub struct TcpCloser {
    stream: TcpStream,
}

impl TcpCloser {
    pub fn close(&self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    /// Дескриптор закрытия из клона сокета (для прерывания ожиданий relay).
    pub fn from_stream(stream: &TcpStream) -> io::Result<Self> {
        Ok(Self {
            stream: stream.try_clone()?,
        })
    }
}

/// Подключиться к адресу и обернуть сокет в [`Duplex`].
pub fn tcp_connect(addr: impl ToSocketAddrs) -> io::Result<(Duplex, TcpCloser)> {
    let stream = TcpStream::connect(addr)?;
    tcp_wrap(stream)
}

/// Принять одно подключение из listener и обернуть сокет в [`Duplex`].
pub fn tcp_accept(listener: &TcpListener) -> io::Result<(Duplex, TcpCloser)> {
    let (stream, _peer) = listener.accept()?;
    tcp_wrap(stream)
}

fn tcp_wrap(stream: TcpStream) -> io::Result<(Duplex, TcpCloser)> {
    // Принятое соединение всегда блокирующее (листенер может быть неблокирующим
    // при accept-цикле агента).
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    let mut rd = stream.try_clone()?;
    let mut wr = stream.try_clone()?;

    let (tx_out, rx_out) = channel::<Vec<u8>>();
    let (tx_in, rx_in) = channel::<Vec<u8>>();

    // Поток чтения: сокет → rx_in.
    std::thread::Builder::new()
        .name("rat-link-read".into())
        .spawn(move || {
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                match rd.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx_in.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            // tx_in умирает здесь → rx_in отдаёт Disconnected.
        })?;

    // Поток записи: tx → сокет.
    std::thread::Builder::new()
        .name("rat-link-write".into())
        .spawn(move || {
            for bytes in rx_out {
                if wr.write_all(&bytes).is_err() {
                    break;
                }
            }
        })?;

    Ok((
        Duplex {
            tx: tx_out,
            rx: rx_in,
        },
        TcpCloser { stream },
    ))
}

// ================== интернет-режим (relay) ==================

/// Прочитать строку протокола relay байт-в-байт, чтобы не «съесть» данные сессии.
fn read_proto_line(stream: &mut TcpStream, max: usize) -> io::Result<String> {
    let mut out = Vec::with_capacity(32);
    let mut b = [0u8; 1];
    loop {
        let n = stream.read(&mut b)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "relay: соединение закрыто",
            ));
        }
        if b[0] == b'\n' {
            break;
        }
        out.push(b[0]);
        if out.len() > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "relay: слишком длинная строка",
            ));
        }
    }
    String::from_utf8(out)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "relay: строка не UTF-8"))
}

fn send_proto_line(stream: &mut TcpStream, line: &str) -> io::Result<()> {
    let mut s = String::with_capacity(line.len() + 1);
    s.push_str(line);
    s.push('\n');
    stream.write_all(s.as_bytes())
}

fn connect_with_timeout(relay: impl ToSocketAddrs, ms: u64) -> io::Result<TcpStream> {
    let sa = relay.to_socket_addrs()?.next().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "relay: пустой адрес сервера")
    })?;
    TcpStream::connect_timeout(&sa, Duration::from_millis(ms))
}

/// Вьюер: подключиться через relay к устройству `id`. После `OK` — сырой режим.
pub fn relay_connect(relay: impl ToSocketAddrs, id: &str) -> io::Result<(Duplex, TcpCloser)> {
    let mut stream = connect_with_timeout(relay, 5000)?;
    stream.set_nodelay(true)?;
    send_proto_line(&mut stream, &format!("CONNECT {id}"))?;
    let line = read_proto_line(&mut stream, 256)?;
    if !line.starts_with("OK") {
        let msg = if line.starts_with("ERR OFFLINE") {
            "устройство не в сети (через сервер)".to_string()
        } else {
            format!("relay: {line}")
        };
        return Err(io::Error::new(io::ErrorKind::ConnectionRefused, msg));
    }
    tcp_wrap(stream)
}

/// Агент: зарегистрировать устройство на relay. После `OK` — ждать [`relay_wait_paired`].
/// Возвращает сокет и дескриптор для прерывания ожидания.
pub fn relay_register(
    relay: impl ToSocketAddrs,
    id: &str,
    name: &str,
) -> io::Result<(TcpStream, TcpCloser)> {
    let mut stream = connect_with_timeout(relay, 5000)?;
    stream.set_nodelay(true)?;
    let name = name.replace(['\n', '\r'], " ");
    send_proto_line(&mut stream, &format!("REG {id} {name}"))?;
    let line = read_proto_line(&mut stream, 256)?;
    if !line.starts_with("OK") {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("relay: {line}"),
        ));
    }
    let closer = TcpCloser::from_stream(&stream)?;
    Ok((stream, closer))
}

/// Агент: дождаться подключения вьюера (`PAIRED`) и перейти в сырой режим.
pub fn relay_wait_paired(stream: TcpStream) -> io::Result<(Duplex, TcpCloser)> {
    let mut stream = stream;
    let line = read_proto_line(&mut stream, 64)?;
    if !line.starts_with("PAIRED") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("relay: {line}"),
        ));
    }
    tcp_wrap(stream)
}

/// Спросить у relay, зарегистрировано ли устройство (короткое соединение).
pub fn relay_who(relay: impl ToSocketAddrs, id: &str) -> io::Result<bool> {
    let mut stream = connect_with_timeout(relay, 1500)?;
    send_proto_line(&mut stream, &format!("WHO {id}"))?;
    let line = read_proto_line(&mut stream, 64)?;
    Ok(line.starts_with("OK"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplex_pair_roundtrip() {
        let (a, b) = duplex_pair();
        a.tx.send(b"hello".to_vec()).unwrap();
        assert_eq!(b.rx.recv().unwrap(), b"hello".to_vec());
        b.tx.send(b"world".to_vec()).unwrap();
        assert_eq!(a.rx.recv().unwrap(), b"world".to_vec());
    }

    #[test]
    fn tcp_loopback_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let (link, _closer) = tcp_connect(addr).unwrap();
            link.tx.send(b"ping".to_vec()).unwrap();
            let got = link.rx.recv().unwrap();
            assert_eq!(got, b"pong".to_vec());
        });
        let (link, closer) = tcp_accept(&listener).unwrap();
        assert_eq!(link.rx.recv().unwrap(), b"ping".to_vec());
        link.tx.send(b"pong".to_vec()).unwrap();
        client.join().unwrap();
        closer.close();
    }
}
