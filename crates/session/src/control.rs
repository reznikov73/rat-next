//! Управляющий канал: helpers для конвертов protobuf (`rat.v0`).

use prost::Message;
use rat_proto::rat::v0::{Envelope, Hello, Ping, Pong, envelope::Body};

/// Сериализовать конверт.
pub fn encode_env(env: &Envelope) -> Vec<u8> {
    let mut b = Vec::with_capacity(64);
    env.encode(&mut b).expect("encode envelope");
    b
}

/// Разобрать конверт (None — битые данные; для диагностики достаточно).
pub fn decode_env(bytes: &[u8]) -> Option<Envelope> {
    Envelope::decode(bytes).ok()
}

pub fn hello(agent_version: &str, capabilities: &[&str]) -> Envelope {
    Envelope {
        ver: 1,
        request_id: 0,
        body: Some(Body::Hello(Hello {
            agent_version: agent_version.to_string(),
            capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
        })),
    }
}

pub fn ping(request_id: u64, nonce: u64) -> Envelope {
    Envelope {
        ver: 1,
        request_id,
        body: Some(Body::Ping(Ping { nonce })),
    }
}

pub fn pong(request_id: u64, nonce: u64) -> Envelope {
    Envelope {
        ver: 1,
        request_id,
        body: Some(Body::Pong(Pong { nonce })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_roundtrip() {
        let env = ping(42, 7);
        let bytes = encode_env(&env);
        let dec = decode_env(&bytes).expect("decode");
        assert_eq!(dec.request_id, 42);
        match dec.body {
            Some(Body::Ping(p)) => assert_eq!(p.nonce, 7),
            other => panic!("неожиданное тело: {other:?}"),
        }
    }

    #[test]
    fn hello_caps() {
        let env = hello("test-0.1", &["video.h264", "mux.v1"]);
        match env.body {
            Some(Body::Hello(h)) => {
                assert_eq!(h.agent_version, "test-0.1");
                assert_eq!(h.capabilities, vec!["video.h264", "mux.v1"]);
            }
            _ => panic!("ожидался Hello"),
        }
    }

    #[test]
    fn pong_echoes_request_id() {
        let env = pong(5, 5);
        let dec = decode_env(&encode_env(&env)).expect("decode");
        assert_eq!(dec.request_id, 5);
        match dec.body {
            Some(Body::Pong(p)) => assert_eq!(p.nonce, 5),
            _ => panic!("ожидался Pong"),
        }
    }

    #[test]
    fn garbage_is_none() {
        assert!(decode_env(&[0xFF, 0x00, 0x13, 0x37]).is_none());
    }
}
