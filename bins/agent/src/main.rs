//! `rat-agent` — агент Rat Next.
//!
//! P2 (пилот): одна видеосессия поверх TCP — захват экрана → H.264 → mux-кадры.
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use rat_session::agent::{AgentConfig, spawn_agent};
    use rat_session::link::tcp_accept;
    use rat_session::stats::{SessionStats, TimeBase};

    let listener =
        TcpListener::bind(&args.listen).map_err(|e| format!("bind {}: {e}", args.listen))?;
    println!("AGENT_LISTENING {}", args.listen);
    let _ = std::io::stdout().flush();

    let (link, closer) = tcp_accept(&listener).map_err(|e| format!("accept: {e}"))?;
    println!("AGENT_CLIENT_CONNECTED");
    let _ = std::io::stdout().flush();

    let base = TimeBase(Instant::now());
    let stop = Arc::new(AtomicBool::new(false));
    let stats = Arc::new(Mutex::new(SessionStats::default()));

    if let Some(secs) = args.smoke {
        let stop_wd = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f32(secs));
            stop_wd.store(true, Ordering::Relaxed);
        });
    }

    let cfg = AgentConfig {
        fps: args.fps,
        bitrate: args.bitrate,
        codec: args.codec,
        monitor: args.monitor,
        ..AgentConfig::default()
    };
    let handles = spawn_agent(base, stop.clone(), stats.clone(), link, cfg)?;
    for h in handles {
        let _ = h.join();
    }
    closer.close();

    let st = stats.lock().unwrap();
    println!("=== AGENT SUMMARY ===");
    println!("{}", st.summary());
    Ok(())
}
