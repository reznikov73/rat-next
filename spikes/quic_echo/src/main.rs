//! Спайк P0: локальный QUIC echo.
//!
//! Проверяет: связка quinn 0.11 + rustls 0.23 (провайдер ring) собирается и
//! работает на Windows/MSVC; цикл «подключение → 5 сообщений → закрытие»
//! проходит на localhost.
//!
//! Запуск (из корня репозитория):
//!   . .\scripts\devshell.ps1
//!   cd spikes\quic_echo
//!   cargo run

use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Провайдер криптографии rustls (ring, без aws-lc-rs).
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls ring provider");

    // Само-подписанный сертификат — ТОЛЬКО для спайка.
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert_der = cert.cert.der().clone();
    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()),
    );

    // --- сервер ---
    let mut server_tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)?;
    server_tls.alpn_protocols = vec![b"rat-spike".to_vec()];

    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_tls)?,
    ));
    let server = quinn::Endpoint::server(server_config, "127.0.0.1:0".parse()?)?;
    let server_addr = server.local_addr()?;
    println!("[server] listening on {server_addr}");

    // --- клиент (принимаем само-подписанный сертификат) ---
    let mut client_tls = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipVerify))
        .with_no_client_auth();
    client_tls.alpn_protocols = vec![b"rat-spike".to_vec()];
    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_tls)?,
    ));

    let server_task = tokio::spawn(async move {
        let incoming = server.accept().await.ok_or("no incoming connection")?;
        let conn = incoming.await?;
        for i in 0..5u64 {
            let (mut send, mut recv) = conn.accept_bi().await?;
            let req = recv.read_to_end(64).await?;
            let expected = format!("ping-{i}");
            assert_eq!(req, expected.as_bytes(), "unexpected request {i}");
            send.write_all(format!("pong-{i}").as_bytes()).await?;
            send.finish()?;
        }
        server.wait_idle().await;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    });

    let mut client = quinn::Endpoint::client("127.0.0.1:0".parse()?)?;
    client.set_default_client_config(client_config);
    let conn = client.connect(server_addr, "localhost")?.await?;
    println!("[client] connected: {server_addr}");

    for i in 0..5u64 {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(format!("ping-{i}").as_bytes()).await?;
        send.finish()?;
        let resp = recv.read_to_end(64).await?;
        assert_eq!(
            resp,
            format!("pong-{i}").as_bytes(),
            "unexpected response {i}"
        );
        println!("[client] ok: ping-{i} -> pong-{i}");
    }

    client.close(0u32.into(), b"done");

    // JoinError и внутреннюю ошибку сервера приводим к общему типу через строку
    // (в спайке важна бинарная «успех/неуспех», детали ошибок всё равно печатаются).
    match server_task.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e.to_string().into()),
        Err(e) => return Err(e.to_string().into()),
    }

    println!("QUIC_OK: 5 echo messages verified");
    Ok(())
}

/// Верификатор, принимающий любой сертификат — только для спайка!
#[derive(Debug)]
struct SkipVerify;

impl rustls::client::danger::ServerCertVerifier for SkipVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
