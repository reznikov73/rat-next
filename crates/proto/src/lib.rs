//! Rat Next — схема управляющего протокола (protobuf v0).
//!
//! Схема — единственный источник правды для управляющего канала
//! (см. docs/ARCHITECTURE_DESIGN_v1.1.md §3.6). Медиа и файловые данные
//! передаются сырыми бинарными кадрами и в эту схему не входят.

/// Сгенерированные типы схемы `rat.v0`.
pub mod rat {
    /// Версия 0 схемы.
    pub mod v0 {
        include!(concat!(env!("OUT_DIR"), "/rat.v0.rs"));
    }
}

#[cfg(test)]
mod tests {
    use super::rat::v0::{Envelope, Ping, envelope::Body};
    use prost::Message;

    #[test]
    fn envelope_roundtrip() {
        let msg = Envelope {
            ver: 1,
            request_id: 42,
            body: Some(Body::Ping(Ping { nonce: 7 })),
        };

        let mut buf = Vec::new();
        msg.encode(&mut buf).expect("encode");
        let decoded = Envelope::decode(buf.as_slice()).expect("decode");

        assert_eq!(decoded.ver, 1);
        assert_eq!(decoded.request_id, 42);
        match decoded.body {
            Some(Body::Ping(p)) => assert_eq!(p.nonce, 7),
            other => panic!("неожиданное тело конверта: {other:?}"),
        }
    }
}
