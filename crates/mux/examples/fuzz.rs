//! fuzz v0: случайные/полу-валидные потоки байт в [`rat_mux::Decoder`].
//!
//! Запуск: `cargo run --release --example fuzz [iterations]`
//! Успех: завершение без паник; выводится скорость и счётчики.

use rat_mux::{Channel, Decoder, Encoder};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

fn main() {
    let iterations: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);

    let mut dec = Decoder::new();
    let mut rng = Lcg(0xDEAD_BEEF_CAFE_1234);
    let mut errors: u64 = 0;
    let mut frames: u64 = 0;
    let mut valid_injected: u64 = 0;

    let t0 = std::time::Instant::now();
    for i in 0..iterations {
        match i % 4 {
            0 => {
                // полностью случайный мусор
                let n = (rng.next() % 96) as usize + 1;
                let mut chunk = vec![0u8; n];
                for b in chunk.iter_mut() {
                    *b = (rng.next() & 0xFF) as u8;
                }
                dec.feed(&chunk);
            }
            1 => {
                // почти-валидный заголовок со случайными полями
                let mut chunk = vec![1u8, (rng.next() & 0xFF) as u8, (rng.next() & 0xFF) as u8];
                chunk.extend_from_slice(&(rng.next() as u32).to_be_bytes());
                chunk.extend_from_slice(&(rng.next() as u32).to_be_bytes());
                dec.feed(&chunk);
            }
            2 => {
                // корректный кадр
                let mut enc = Encoder::new();
                let n = (rng.next() % 256) as usize;
                let payload = vec![(rng.next() & 0xFF) as u8; n];
                enc.push(Channel::Video, 0, &payload);
                dec.feed(&enc.take_bytes());
                valid_injected += 1;
            }
            _ => {}
        }

        loop {
            match dec.poll_frame() {
                Ok(Some(f)) => {
                    frames += 1;
                    std::hint::black_box(f.payload.len());
                }
                Ok(None) => break,
                Err(_) => {
                    errors += 1;
                    dec.reset();
                    break;
                }
            }
        }
    }

    let dt = t0.elapsed().as_secs_f64();
    println!(
        "FUZZ_OK iterations={iterations} frames={frames} errors={errors} valid_injected={valid_injected} \
         {:.0} iter/s in {:.2}s",
        iterations as f64 / dt,
        dt
    );
    println!("FUZZ_DONE_NO_PANICS");
}
