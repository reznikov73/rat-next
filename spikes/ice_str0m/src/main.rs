//! Спайк P0-Б: ICE connectivity между двумя локальными агентами.
//!
//! Используется крейт `is` — это ICE-слой из str0m 0.24 (str0m ре-экспортирует его
//! как `str0m::ice`; в продукте пойдёт через str0m либо напрямую `is`).
//!
//! Сценарий: два IceAgent (controlling / controlled) на loopback-адресах
//! 127.0.0.1:41000 и 127.0.0.1:41001. Кандидаты и креды передаются напрямую
//! (как это делал бы signaling-сервер), Transmit'ы доставляются in-memory
//! (как это делал бы UDP-транспорт). Успех: оба агента дошли до Connected.
//!
//! Запуск: cargo run

use std::time::{Duration, Instant};

use is::{Candidate, IceAgent, IceConnectionState};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("ICE spike start (crate `is`)");
    let t0 = Instant::now();

    let mut a = IceAgent::new(is::IceCreds::new());
    let mut b = IceAgent::new(is::IceCreds::new());
    a.set_controlling(true);
    b.set_controlling(false);

    // Обмен кредами (в реальном сигналинге — через signaling-сервер).
    let a_creds = a.local_credentials().clone();
    let b_creds = b.local_credentials().clone();
    a.set_remote_credentials(b_creds);
    b.set_remote_credentials(a_creds);

    // Host-кандидаты на loopback.
    let a_addr: std::net::SocketAddr = "127.0.0.1:41000".parse()?;
    let b_addr: std::net::SocketAddr = "127.0.0.1:41001".parse()?;
    let a_cand = Candidate::host(a_addr, is::Protocol::Udp)?;
    let b_cand = Candidate::host(b_addr, is::Protocol::Udp)?;
    let _ = a.add_local_candidate(a_cand.clone());
    let _ = b.add_local_candidate(b_cand.clone());
    a.add_remote_candidate(b_cand);
    b.add_remote_candidate(a_cand);
    println!("candidates exchanged: a={a_addr} b={b_addr}");

    // Прогон connectivity checks: Transmit'ы передаём напрямую между агентами.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut transmits = 0u64;
    loop {
        let now = Instant::now();
        if now > deadline {
            println!("ICE_TIMEOUT: a={:?} b={:?}", a.state(), b.state());
            return Err("ICE timeout".into());
        }

        a.handle_timeout(now);
        b.handle_timeout(now);

        // a -> b
        while let Some(t) = a.poll_transmit() {
            let msg = is::stun::StunMessage::parse(&t.contents[..])?;
            let packet = is::stun::StunPacket {
                proto: t.proto,
                source: t.source,
                destination: t.destination,
                message: msg,
            };
            b.handle_packet(now, packet);
            transmits += 1;
        }
        // b -> a
        while let Some(t) = b.poll_transmit() {
            let msg = is::stun::StunMessage::parse(&t.contents[..])?;
            let packet = is::stun::StunPacket {
                proto: t.proto,
                source: t.source,
                destination: t.destination,
                message: msg,
            };
            a.handle_packet(now, packet);
            transmits += 1;
        }

        for ev in std::iter::from_fn(|| a.poll_event()) {
            println!("[a] event: {ev:?}");
        }
        for ev in std::iter::from_fn(|| b.poll_event()) {
            println!("[b] event: {ev:?}");
        }

        let a_done = matches!(
            a.state(),
            IceConnectionState::Connected | IceConnectionState::Completed
        );
        let b_done = matches!(
            b.state(),
            IceConnectionState::Connected | IceConnectionState::Completed
        );
        if a_done && b_done {
            println!(
                "ICE_OK: both connected in {:?} ({} transmits); states: a={:?} b={:?}",
                t0.elapsed(),
                transmits,
                a.state(),
                b.state()
            );
            return Ok(());
        }

        std::thread::sleep(Duration::from_millis(5));
    }
}
