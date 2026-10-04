//! `rat-agent` — агент Rat Next.
//!
//! P3 (пилот): видеосессии поверх TCP в цикле — агент обслуживает подключения
//! подряд (после отключения вьюера снова ждёт следующий), Ctrl+C — остановить.
//! Продуктовый запуск — служба Windows (Session 0), см. docs/ARCHITECTURE_DESIGN_v1.1.md §3.
//!
//! Запуск:
//! ```text
//! cargo run -p rat-agent --release --features ffmpeg -- --listen 127.0.0.1:55140
//! cargo run -p rat-agent --release --features ffmpeg -- --listen 127.0.0.1:55140 --smoke=20
//! ```

#[cfg_attr(not(feature = "ffmpeg"), allow(dead_code))]
struct Args {
    listen: String,
    fps: u32,
    bitrate: i64,
    codec: String,
    monitor: u32,
    smoke: Option<f32>,
}

fn parse_args() -> Args {
    let mut a = Args {
        listen: "127.0.0.1:55140".to_string(),
        fps: 30,
        bitrate: 8_000_000,
        codec: std::env::var("VIDEO_CODEC").unwrap_or_else(|_| "h264_qsv".to_string()),
        monitor: 0,
        smoke: None,
    };
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        let cur = args[i].as_str();
        match cur {
            "--listen" if i + 1 < args.len() => {
                a.listen = args[i + 1].clone();
                i += 2;
            }
            "--fps" if i + 1 < args.len() => {
                a.fps = args[i + 1].parse().unwrap_or(30);
                i += 2;
            }
            "--bitrate" if i + 1 < args.len() => {
                a.bitrate = args[i + 1].parse().unwrap_or(8_000_000);
                i += 2;
            }
            "--codec" if i + 1 < args.len() => {
                a.codec = args[i + 1].clone();
                i += 2;
            }
            "--monitor" if i + 1 < args.len() => {
                a.monitor = args[i + 1].parse().unwrap_or(0);
                i += 2;
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
    eprintln!("rat-agent: соберите с --features ffmpeg (пилотный кодек-путь P2)");
    std::process::exit(3);
}

#[cfg(feature = "ffmpeg")]
fn main() {
    let args = parse_args();
    if let Err(e) = run(args) {
        eprintln!("agent error: {e}");
        std::process::exit(1);
    }
}

#[cfg(feature = "ffmpeg")]
fn run(args: Args) -> Result<(), String> {
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use rat_session::agent::{AgentConfig, spawn_agent};
    use rat_session::link::tcp_accept;
    use rat_session::stats::{SessionStats, TimeBase};

    let listener =
        TcpListener::bind(&args.listen).map_err(|e| format!("bind {}: {e}", args.listen))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {e}"))?;
    println!("AGENT_LISTENING {}", args.listen);
    if args.listen.starts_with("127.") || args.listen.starts_with("localhost") {
        println!(
            "[agent] ВНИМАНИЕ: слушаю только локально — из сети этот порт недоступен. \
             Для доступа из сети: --listen 0.0.0.0:55140 (или scripts\\run-agent-lan.cmd)"
        );
    }
    println!("[agent] Жду подключения. Ctrl+C — остановить.");
    let _ = std::io::stdout().flush();

    let started = Instant::now();
    loop {
        if let Some(secs) = args.smoke
            && started.elapsed().as_secs_f32() >= secs
        {
            println!("AGENT_STOPPED (smoke-таймер истёк)");
            break;
        }
        match tcp_accept(&listener) {
            Ok((link, closer)) => {
                println!("AGENT_CLIENT_CONNECTED");
                let _ = std::io::stdout().flush();

                let base = TimeBase(Instant::now());
                let stop = Arc::new(AtomicBool::new(false));
                let stats = Arc::new(Mutex::new(SessionStats::default()));
                let cfg = AgentConfig {
                    fps: args.fps,
                    bitrate: args.bitrate,
                    codec: args.codec.clone(),
                    monitor: args.monitor,
                    ..AgentConfig::default()
                };
                match spawn_agent(base, stop.clone(), stats.clone(), link, cfg) {
                    Ok(handles) => {
                        for h in handles {
                            let _ = h.join();
                        }
                        closer.close();
                        let st = stats.lock().unwrap();
                        println!("=== AGENT SUMMARY ===");
                        println!("{}", st.summary());
                    }
                    Err(e) => {
                        closer.close();
                        eprintln!("[agent] ошибка старта сессии: {e}");
                    }
                }
                println!("AGENT_WAITING_FOR_CLIENT");
                let _ = std::io::stdout().flush();
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("accept: {e}")),
        }
    }
    Ok(())
}
