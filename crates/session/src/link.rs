//! Линк сессии: байтовый дуплекс между сторонами.
//!
//! Два варианта: in-memory (`duplex_pair`, для тестов) и TCP (`tcp_connect` /
//! `tcp_accept`, первый сетевой транспорт пилота). Mux транспорт-агностичен —
//! позже сюда встанет QUIC.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, Sender, channel};

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
