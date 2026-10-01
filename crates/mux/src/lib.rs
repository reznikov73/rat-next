//! Rat Next — мультиплексирование каналов (v1, sans-IO).
//!
//! Формат кадра:
//! `[ver u8][channel u8][flags u8][seq u32 BE][len u32 BE][payload]`
//!
//! - `ver` — версия схемы мультиплексирования (сейчас 1);
//! - `channel` — логический канал (control/input/cursor/video/files/terminal/clipboard);
//! - `flags` — битовые флаги (KEYFRAME / DROPPABLE);
//! - `seq` — сквозной номер кадра (диагностика, потери на канале);
//! - `len` — длина payload (не более [`MAX_PAYLOAD`]).
//!
//! Sans-IO: [`Encoder`] пишет байты во внутренний буфер (его забирает вызывающий и
//! отправляет по любому надёжному упорядоченному транспорту), [`Decoder`] принимает
//! байты через [`Decoder::feed`] и выдаёт готовые кадры через [`Decoder::poll_frame`].

pub const PROTOCOL_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 11;
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

/// Логические каналы. Приоритет канала — см. [`Channel::priority`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Channel {
    Control = 0,
    Input = 1,
    Cursor = 2,
    Video = 3,
    Files = 4,
    Terminal = 5,
    Clipboard = 6,
}

impl Channel {
    pub fn from_u8(v: u8) -> Result<Self, MuxError> {
        match v {
            0 => Ok(Channel::Control),
            1 => Ok(Channel::Input),
            2 => Ok(Channel::Cursor),
            3 => Ok(Channel::Video),
            4 => Ok(Channel::Files),
            5 => Ok(Channel::Terminal),
            6 => Ok(Channel::Clipboard),
            other => Err(MuxError::BadChannel(other)),
        }
    }

    /// Приоритет канала: 0 — высший.
    pub fn priority(self) -> u8 {
        match self {
            Channel::Control | Channel::Input => 0,
            Channel::Cursor | Channel::Video => 1,
            Channel::Files | Channel::Terminal => 2,
            Channel::Clipboard => 3,
        }
    }
}

/// Флаги кадра.
pub mod flags {
    /// Ключевой кадр видео (IDR).
    pub const KEYFRAME: u8 = 0x01;
    /// Кадр можно отбрасывать при перегрузке (например, видео).
    pub const DROPPABLE: u8 = 0x02;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MuxError {
    BadVersion(u8),
    BadChannel(u8),
    PayloadTooLarge(u32),
}

impl std::fmt::Display for MuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MuxError::BadVersion(v) => write!(f, "неподдерживаемая версия протокола: {v}"),
            MuxError::BadChannel(v) => write!(f, "неизвестный канал: {v}"),
            MuxError::PayloadTooLarge(v) => write!(f, "payload слишком большой: {v}"),
        }
    }
}

impl std::error::Error for MuxError {}

/// Разобранный кадр (заимствует буфер декодера — используйте до следующего вызова).
#[derive(Debug, PartialEq, Eq)]
pub struct Frame<'a> {
    pub channel: Channel,
    pub flags: u8,
    pub seq: u32,
    pub payload: &'a [u8],
}

/// Кодировщик кадров (накапливает байты во внутреннем буфере).
#[derive(Default)]
pub struct Encoder {
    out: Vec<u8>,
    seq: u32,
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Добавить кадр во внутренний буфер.
    pub fn push(&mut self, channel: Channel, flags: u8, payload: &[u8]) {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        self.out.push(PROTOCOL_VERSION);
        self.out.push(channel as u8);
        self.out.push(flags);
        self.out.extend_from_slice(&seq.to_be_bytes());
        self.out
            .extend_from_slice(&(payload.len() as u32).to_be_bytes());
        self.out.extend_from_slice(payload);
    }

    /// Забрать накопленные байты (буфер очищается).
    pub fn take_bytes(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    pub fn is_empty(&self) -> bool {
        self.out.is_empty()
    }

    pub fn seq(&self) -> u32 {
        self.seq
    }
}

/// Декодировщик кадров (принимает произвольно нарезанные байты).
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    read_pos: usize,
    pub frames_decoded: u64,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Добавить принятые байты.
    pub fn feed(&mut self, data: &[u8]) {
        // Компактация: если прочитанное начало достаточно велико — сдвигаем.
        if self.read_pos > 0 && (self.read_pos == self.buf.len() || self.read_pos > 64 * 1024) {
            self.buf.drain(..self.read_pos);
            self.read_pos = 0;
        }
        self.buf.extend_from_slice(data);
    }

    /// Сколько непрочитанных байт в буфере.
    pub fn buffered_len(&self) -> usize {
        self.buf.len() - self.read_pos
    }

    /// Разобрать следующий кадр.
    ///
    /// `Ok(None)` — данных пока недостаточно; `Err` — поток битый
    /// (в продукте это означает разрыв соединения; для продолжения вызвать [`Decoder::reset`]).
    pub fn poll_frame(&mut self) -> Result<Option<Frame<'_>>, MuxError> {
        let start = self.read_pos;
        if self.buf.len() - start < HEADER_LEN {
            return Ok(None);
        }
        let ver = self.buf[start];
        if ver != PROTOCOL_VERSION {
            return Err(MuxError::BadVersion(ver));
        }
        let channel = Channel::from_u8(self.buf[start + 1])?;
        let flags = self.buf[start + 2];
        let seq = u32::from_be_bytes([
            self.buf[start + 3],
            self.buf[start + 4],
            self.buf[start + 5],
            self.buf[start + 6],
        ]);
        let len = u32::from_be_bytes([
            self.buf[start + 7],
            self.buf[start + 8],
            self.buf[start + 9],
            self.buf[start + 10],
        ]) as usize;
        if len > MAX_PAYLOAD {
            return Err(MuxError::PayloadTooLarge(len as u32));
        }
        let payload_start = start + HEADER_LEN;
        let payload_end = payload_start + len;
        if payload_end > self.buf.len() {
            return Ok(None);
        }
        self.read_pos = payload_end;
        self.frames_decoded += 1;
        Ok(Some(Frame {
            channel,
            flags,
            seq,
            payload: &self.buf[payload_start..payload_end],
        }))
    }

    /// Полный сброс буфера (после ошибок/переподключения).
    pub fn reset(&mut self) {
        self.buf.clear();
        self.read_pos = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_frames(mut dec: Decoder) -> Vec<(Channel, u8, u32, Vec<u8>)> {
        let mut out = Vec::new();
        while let Some(f) = dec.poll_frame().expect("valid stream") {
            out.push((f.channel, f.flags, f.seq, f.payload.to_vec()));
        }
        out
    }

    #[test]
    fn roundtrip_split_by_bytes() {
        let mut enc = Encoder::new();
        enc.push(Channel::Control, 0, b"hello");
        enc.push(
            Channel::Video,
            flags::KEYFRAME | flags::DROPPABLE,
            &[0u8; 1000],
        );
        enc.push(Channel::Input, 0, &[1, 2, 3, 4, 5]);
        let bytes = enc.take_bytes();

        let mut dec = Decoder::new();
        let mut got = Vec::new();
        for b in bytes {
            dec.feed(&[b]);
            while let Some(f) = dec.poll_frame().expect("valid stream") {
                got.push((f.channel, f.flags, f.seq, f.payload.to_vec()));
            }
        }
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], (Channel::Control, 0, 0, b"hello".to_vec()));
        assert_eq!(got[1].0, Channel::Video);
        assert_eq!(got[1].1, flags::KEYFRAME | flags::DROPPABLE);
        assert_eq!(got[1].2, 1);
        assert_eq!(got[1].3.len(), 1000);
        assert_eq!(got[2], (Channel::Input, 0, 2, vec![1, 2, 3, 4, 5]));
    }

    #[test]
    fn multiple_frames_one_feed() {
        let mut enc = Encoder::new();
        enc.push(Channel::Control, 0, b"a");
        enc.push(Channel::Control, 0, b"bb");
        enc.push(Channel::Control, 0, b"ccc");
        let bytes = enc.take_bytes();

        let mut dec = Decoder::new();
        dec.feed(&bytes);
        let frames = collect_frames(dec);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[2].3, b"ccc".to_vec());
    }

    #[test]
    fn bad_version_rejected() {
        let mut dec = Decoder::new();
        let mut bytes = vec![2u8]; // версия 2
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        dec.feed(&bytes);
        assert_eq!(dec.poll_frame(), Err(MuxError::BadVersion(2)));
    }

    #[test]
    fn bad_channel_rejected() {
        let mut dec = Decoder::new();
        let bytes = [PROTOCOL_VERSION, 250, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        dec.feed(&bytes);
        assert_eq!(dec.poll_frame(), Err(MuxError::BadChannel(250)));
    }

    #[test]
    fn too_large_rejected_even_without_payload() {
        let mut dec = Decoder::new();
        let big = (MAX_PAYLOAD as u32) + 1;
        let mut bytes = vec![PROTOCOL_VERSION, Channel::Video as u8, 0];
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&big.to_be_bytes());
        dec.feed(&bytes);
        assert_eq!(dec.poll_frame(), Err(MuxError::PayloadTooLarge(big)));
    }

    /// Детерминированный xorshift64 (без внешних зависимостей).
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

    #[test]
    fn garbage_does_not_panic_or_grow_unbounded() {
        let mut dec = Decoder::new();
        let mut rng = Lcg(0x1234_5678_9abc_def0);
        for i in 0..20_000u32 {
            // случайный мусор
            let n = (rng.next() % 64) as usize + 1;
            let mut chunk = vec![0u8; n];
            for b in chunk.iter_mut() {
                *b = (rng.next() & 0xFF) as u8;
            }
            dec.feed(&chunk);
            // изредка подмешиваем корректный кадр
            if i % 500 == 0 {
                let mut enc = Encoder::new();
                enc.push(Channel::Video, 0, &[7u8; 32]);
                dec.feed(&enc.take_bytes());
            }
            loop {
                match dec.poll_frame() {
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => {
                        dec.reset();
                        break;
                    }
                }
            }
            assert!(
                dec.buffered_len() <= 64 * 1024 + 128,
                "буфер не должен расти"
            );
        }
    }
}
