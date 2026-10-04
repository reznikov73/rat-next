//! Метрики сессии (общие для агента и вьюера).

use std::time::Instant;

/// Общая точка отсчёта времени сессии.
#[derive(Clone, Copy)]
pub struct TimeBase(pub Instant);

/// Микросекунды с начала сессии.
pub fn now_us(base: &TimeBase) -> u64 {
    base.0.elapsed().as_micros() as u64
}

#[derive(Default)]
pub struct SessionStats {
    // агент
    pub captured: u64,
    pub sent_pkts: u64,
    pub sent_bytes: u64,
    /// Кадры, отброшенные при занятом энкодере (EAGAIN; политика «не копить»).
    pub agent_dropped: u64,
    pub agent_errors: u64,
    // вьюер
    pub shown: u64,
    /// Кадры, отброшенные при перегрузке приёмника UI («самый свежий кадр»).
    pub viewer_dropped: u64,
    pub viewer_errors: u64,
    pub fps: f32,
    pub codec: String,
    pub recv_bytes: u64,
    pub control_msgs: u64,
    // ввод/курсор (P3)
    /// Событий ввода отправлено вьюером.
    pub input_sent: u64,
    /// Событий ввода проинъектировано агентом.
    pub input_recv: u64,
    /// Обновлений курсора отправлено агентом.
    pub cursor_sent: u64,
    /// Успешных переключений монитора «на лету».
    pub capture_switches: u64,
    // выборки (мс)
    pub e2e_ms: Vec<f32>,
    pub rtt_ms: Vec<f32>,
    pub last_error: String,
}

impl SessionStats {
    pub fn push_sample(v: &mut Vec<f32>, x: f32) {
        v.push(x);
        if v.len() > 600 {
            v.remove(0);
        }
    }

    pub fn avg(v: &[f32]) -> f32 {
        if v.is_empty() {
            return 0.0;
        }
        v.iter().sum::<f32>() / v.len() as f32
    }

    pub fn pct(v: &[f32], p: f32) -> f32 {
        if v.is_empty() {
            return 0.0;
        }
        let mut s = v.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        s[((s.len() - 1) as f32 * p) as usize]
    }

    pub fn summary(&self) -> String {
        format!(
            "frames: captured={} sent_pkts={} shown={} drops(agent={} viewer={})\n\
             bytes: sent={} recv={}\n\
             fps={:.1}  codec={}  control_msgs={}\n\
             input: sent={} injected={} cursor={} switches={}\n\
             e2e:   avg {:.2}ms  p50 {:.2}ms  p95 {:.2}ms\n\
             rtt:   avg {:.2}ms  p95 {:.2}ms\n\
             errors: agent={} viewer={}{}",
            self.captured,
            self.sent_pkts,
            self.shown,
            self.agent_dropped,
            self.viewer_dropped,
            self.sent_bytes,
            self.recv_bytes,
            self.fps,
            if self.codec.is_empty() {
                "-"
            } else {
                &self.codec
            },
            self.control_msgs,
            self.input_sent,
            self.input_recv,
            self.cursor_sent,
            self.capture_switches,
            Self::avg(&self.e2e_ms),
            Self::pct(&self.e2e_ms, 0.5),
            Self::pct(&self.e2e_ms, 0.95),
            Self::avg(&self.rtt_ms),
            Self::pct(&self.rtt_ms, 0.95),
            self.agent_errors,
            self.viewer_errors,
            if self.last_error.is_empty() {
                String::new()
            } else {
                format!("\nlast_error: {}", self.last_error)
            }
        )
    }
}
