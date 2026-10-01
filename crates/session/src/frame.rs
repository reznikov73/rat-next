//! Кадры видеоканала: мета `[frame_id u64 BE][ts_capture_us u64 BE]` + Annex-B H.264.

/// Длина метаданных видеокадра (frame_id + ts захвата).
pub const META_LEN: usize = 16;

/// Сложить видеопакет: мета + Annex-B данные (добавляется к `out`).
pub fn pack(out: &mut Vec<u8>, frame_id: u64, ts_us: u64, annexb: &[u8]) {
    out.reserve(META_LEN + annexb.len());
    out.extend_from_slice(&frame_id.to_be_bytes());
    out.extend_from_slice(&ts_us.to_be_bytes());
    out.extend_from_slice(annexb);
}

/// Разобрать payload видеокадра: (frame_id, ts_us, annexb-данные).
pub fn parse(payload: &[u8]) -> Option<(u64, u64, &[u8])> {
    if payload.len() < META_LEN {
        return None;
    }
    let id = u64::from_be_bytes(payload[0..8].try_into().ok()?);
    let ts = u64::from_be_bytes(payload[8..16].try_into().ok()?);
    Some((id, ts, &payload[META_LEN..]))
}

/// Есть ли IDR-слайс (NAL type 5) в первых 2 КиБ потока — грубая проверка
/// для флага [`rat_mux::flags::KEYFRAME`].
pub fn is_idr(data: &[u8]) -> bool {
    let scan = data.len().min(2048);
    let mut i = 0;
    while i + 4 < scan {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
            let nal_type = data[i + 4] & 0x1F;
            if nal_type == 5 {
                return true;
            }
            i += 4;
        } else {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_parse_roundtrip() {
        let mut buf = vec![1u8, 2, 3]; // «чужие» байты перед кадром
        pack(&mut buf, 77, 123_456_789, &[9, 8, 7]);
        let (id, ts, data) = parse(&buf[3..]).expect("parse");
        assert_eq!((id, ts), (77, 123_456_789));
        assert_eq!(data, &[9, 8, 7]);
    }

    #[test]
    fn parse_short_is_none() {
        assert!(parse(&[0u8; 15]).is_none());
        assert!(parse(&[]).is_none());
    }

    #[test]
    fn idr_detection() {
        assert!(is_idr(&[0, 0, 0, 1, 0x65, 0x88])); // IDR-слайс
        assert!(!is_idr(&[0, 0, 0, 1, 0x41, 0x88])); // P-слайс
        assert!(!is_idr(&[0, 0, 0, 1, 0x67, 0, 0, 1, 0x41])); // SPS + P
        assert!(is_idr(&[0, 0, 0, 1, 0x67, 0, 0, 0, 1, 0x65, 1])); // SPS + IDR
        assert!(!is_idr(&[]));
        assert!(!is_idr(&[0, 0, 0])); // короткий хвост
    }
}
