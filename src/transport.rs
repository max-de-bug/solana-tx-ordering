use anyhow::{Context, Result};
use quinn::crypto::rustls::QuicClientConfig;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Solana TPU QUIC ALPN. This is the same protocol identifier used by Agave's
/// TPU QUIC server.
pub const ALPN_TPU_PROTOCOL_ID: &[u8] = b"solana-tpu";

/// Direct client for a validator's `tpuQuic` endpoint.
///
/// The connection is established once and reused. Transactions are written in
/// dispatch order, each on its own unidirectional QUIC stream, matching the
/// wire format expected by the TPU.
pub struct TpuSender {
    addr: SocketAddr,
    endpoint: quinn::Endpoint,
    connection: Arc<Mutex<Option<quinn::Connection>>>,
}

impl TpuSender {
    pub fn new(addr: SocketAddr) -> Result<Self> {
        let cert = rcgen::generate_simple_self_signed(vec!["solana-tpu".to_string()])
            .context("generate client certificate")?;
        let cert_der = CertificateDer::from(cert.cert.der().to_vec());
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut crypto = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .context("select TLS protocol versions")?
            .dangerous()
            .with_custom_certificate_verifier(SkipServerVerification::new())
            .with_client_auth_cert(vec![cert_der], key_der)
            .context("configure QUIC client certificate")?;
        crypto.alpn_protocols = vec![ALPN_TPU_PROTOCOL_ID.to_vec()];

        let client_config = quinn::ClientConfig::new(Arc::new(
            QuicClientConfig::try_from(crypto).context("build QUIC client config")?,
        ));

        let mut endpoint =
            quinn::Endpoint::client("0.0.0.0:0".parse()?).context("bind local QUIC endpoint")?;
        endpoint.set_default_client_config(client_config);

        Ok(Self {
            addr,
            endpoint,
            connection: Arc::new(Mutex::new(None)),
        })
    }

    #[allow(dead_code)]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Establish (or refresh) the QUIC connection. Call this before the timed
    /// send window so the handshake is not part of the measured latency.
    pub async fn warmup(&self) -> Result<()> {
        let connecting = self
            .endpoint
            .connect(self.addr, "solana-tpu")
            .with_context(|| format!("start QUIC connection to {}", self.addr))?;
        let connection = connecting
            .await
            .with_context(|| format!("complete QUIC handshake with {}", self.addr))?;
        debug!("QUIC connection to {} established", self.addr);
        *self.connection.lock().await = Some(connection);
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn is_connected(&self) -> bool {
        self.connection
            .lock()
            .await
            .as_ref()
            .map(|c| c.close_reason().is_none())
            .unwrap_or(false)
    }

    async fn connection(&self) -> Result<quinn::Connection> {
        if let Some(connection) = self.connection.lock().await.clone() {
            if connection.close_reason().is_none() {
                return Ok(connection);
            }
        }
        self.warmup().await?;
        self.connection
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("QUIC connection was not established"))
    }

    /// Send one serialized transaction on a fresh unidirectional stream.
    pub async fn send_wire(&self, wire: &[u8]) -> Result<()> {
        let connection = self.connection().await?;
        let mut send = connection
            .open_uni()
            .await
            .context("open QUIC send stream")?;
        send.write_all(wire)
            .await
            .context("write transaction to QUIC stream")?;
        send.finish().context("finish QUIC send stream")?;
        Ok(())
    }

    /// Send a batch in slice order. Returns one result per input so callers can
    /// record partial failures instead of silently dropping probes.
    pub async fn send_batch(&self, wires: &[Vec<u8>]) -> Vec<Result<()>> {
        let mut results = Vec::with_capacity(wires.len());
        for (idx, wire) in wires.iter().enumerate() {
            let mut result = self.send_wire(wire).await;
            if result.is_err() {
                warn!(
                    "TPU send failed for probe {} to {}: {:?}; attempting one reconnect",
                    idx,
                    self.addr,
                    result.as_ref().err()
                );
                if self.warmup().await.is_ok() {
                    result = self.send_wire(wire).await;
                }
            }
            results.push(result);
        }
        results
    }

    pub async fn close(&self) {
        if let Some(connection) = self.connection.lock().await.take() {
            connection.close(0u32.into(), b"campaign complete");
        }
        self.endpoint.close(0u32.into(), b"campaign complete");
    }
}

pub fn parse_tpu_addr(raw: &str) -> Result<SocketAddr> {
    SocketAddr::from_str(raw.trim())
        .with_context(|| format!("invalid tpuQuic address {raw:?}; expected ip:port"))
}

#[derive(Debug)]
struct SkipServerVerification(Arc<CryptoProvider>);

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self(Arc::new(rustls::crypto::ring::default_provider())))
    }
}

impl ServerCertVerifier for SkipServerVerification {
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }

    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Solana TPU uses self-signed certificates. Authentication of the
        // validator is performed by the cluster's stake-weighted QUIC layer,
        // not by the public web PKI.
        Ok(ServerCertVerified::assertion())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipv4_tpu_address() {
        let addr = parse_tpu_addr("64.130.57.215:8002").unwrap();
        assert_eq!(addr.port(), 8002);
    }

    #[test]
    fn rejects_missing_port() {
        assert!(parse_tpu_addr("64.130.57.215").is_err());
    }

    #[tokio::test]
    async fn client_config_builds() {
        // This exercises the certificate, TLS, and QUIC configuration path
        // without making a network connection.
        let addr: SocketAddr = "127.0.0.1:9".parse().unwrap();
        let sender = TpuSender::new(addr).unwrap();
        assert_eq!(sender.addr().port(), 9);
    }
}
