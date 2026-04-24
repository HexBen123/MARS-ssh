use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
#[cfg(windows)]
use tokio_native_tls::TlsConnector as ClientTlsConnector;
#[cfg(not(windows))]
use tokio_rustls::rustls::{
    self,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, Error, SignatureScheme,
};
use tokio_rustls::rustls::{pki_types::PrivateKeyDer, ServerConfig};
use tokio_rustls::TlsAcceptor;
#[cfg(not(windows))]
use tokio_rustls::TlsConnector as ClientTlsConnector;

use crate::config::AgentConfig;

#[cfg(windows)]
pub type ClientTlsStream = tokio_native_tls::TlsStream<TcpStream>;
#[cfg(not(windows))]
pub type ClientTlsStream = tokio_rustls::client::TlsStream<TcpStream>;

pub fn parse_fingerprint(value: &str) -> Result<[u8; 32]> {
    let mut normalized = value.trim().to_ascii_lowercase();
    if let Some(rest) = normalized.strip_prefix("sha256:") {
        normalized = rest.to_string();
    }
    let normalized = normalized.replace(':', "");
    let decoded = hex::decode(&normalized).context("decode fingerprint")?;
    if decoded.len() != 32 {
        anyhow::bail!("fingerprint must be 32 bytes, got {}", decoded.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&decoded);
    Ok(out)
}

pub async fn connect_pinned_tls(cfg: &AgentConfig) -> Result<ClientTlsStream> {
    let derived_host = relay_host(&cfg.relay);
    let server_name = cfg
        .server_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(derived_host.as_deref())
        .ok_or_else(|| anyhow::anyhow!("server_name is empty and relay host cannot be derived"))?
        .to_string();
    let want = parse_fingerprint(&cfg.fingerprint)?;
    let tcp = timeout(Duration::from_secs(10), TcpStream::connect(&cfg.relay))
        .await
        .context("tcp connect timeout")?
        .with_context(|| format!("dial {}", cfg.relay))?;

    let tls = connect_unverified_tls_stream(tcp, &server_name, Duration::from_secs(10)).await?;

    verify_peer_fingerprint(&tls, want)?;
    Ok(tls)
}

pub fn verify_peer_fingerprint(tls: &ClientTlsStream, want: [u8; 32]) -> Result<()> {
    let der = peer_leaf_der(tls)?;
    let got = Sha256::digest(&der);
    if got.as_slice() != want {
        anyhow::bail!(
            "certificate fingerprint mismatch: got sha256:{}",
            hex::encode(got)
        );
    }
    Ok(())
}

pub async fn fetch_fingerprint(addr: &str, server_name: &str) -> Result<String> {
    let tls = connect_unverified_tls(addr, server_name, Duration::from_secs(10)).await?;
    Ok(fingerprint_from_der(&peer_leaf_der(&tls)?))
}

pub async fn connect_unverified_tls(
    addr: &str,
    server_name: &str,
    connect_timeout: Duration,
) -> Result<ClientTlsStream> {
    let tcp = timeout(connect_timeout, TcpStream::connect(addr))
        .await
        .context("tcp connect timeout")?
        .with_context(|| format!("dial {addr}"))?;
    connect_unverified_tls_stream(tcp, server_name, connect_timeout).await
}

async fn connect_unverified_tls_stream(
    tcp: TcpStream,
    server_name: &str,
    handshake_timeout: Duration,
) -> Result<ClientTlsStream> {
    #[cfg(windows)]
    {
        let mut builder = tokio_native_tls::native_tls::TlsConnector::builder();
        builder
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .min_protocol_version(Some(tokio_native_tls::native_tls::Protocol::Tlsv12));
        let connector = ClientTlsConnector::from(builder.build().context("build tls")?);
        timeout(handshake_timeout, connector.connect(server_name, tcp))
            .await
            .context("tls handshake timeout")?
            .context("tls handshake")
    }

    #[cfg(not(windows))]
    {
        let name = ServerName::try_from(server_name.to_string())
            .map_err(|_| anyhow::anyhow!("invalid TLS server name: {server_name}"))?;
        let config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
            .with_no_client_auth();
        let connector = ClientTlsConnector::from(Arc::new(config));
        timeout(handshake_timeout, connector.connect(name, tcp))
            .await
            .context("tls handshake timeout")?
            .context("tls handshake")
    }
}

pub fn fingerprint_from_file(cert_file: &str) -> Result<String> {
    let certs = load_certs(cert_file)?;
    let cert = certs
        .first()
        .ok_or_else(|| anyhow::anyhow!("cert file {cert_file} contains no CERTIFICATE block"))?;
    Ok(fingerprint_from_der(cert.as_ref()))
}

fn fingerprint_from_der(der: &[u8]) -> String {
    let sum = Sha256::digest(der);
    format!("sha256:{}", hex::encode(sum))
}

#[cfg(not(windows))]
#[derive(Debug)]
struct NoCertificateVerification;

#[cfg(not(windows))]
impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(windows)]
fn peer_leaf_der(tls: &ClientTlsStream) -> Result<Vec<u8>> {
    let cert = tls
        .get_ref()
        .peer_certificate()
        .context("get peer certificate")?
        .ok_or_else(|| anyhow::anyhow!("server presented no certificate"))?;
    cert.to_der().context("encode peer certificate der")
}

#[cfg(not(windows))]
fn peer_leaf_der(tls: &ClientTlsStream) -> Result<Vec<u8>> {
    let cert = tls
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certs| certs.first())
        .ok_or_else(|| anyhow::anyhow!("server presented no certificate"))?;
    Ok(cert.as_ref().to_vec())
}

pub fn relay_host(relay: &str) -> Option<String> {
    relay.rsplit_once(':').map(|(host, _)| host.to_string())
}

pub fn load_server_tls_acceptor(cert_file: &str, key_file: &str) -> Result<TlsAcceptor> {
    let certs = load_certs(cert_file)?;
    let key = load_private_key(key_file)?;
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("build server tls config")?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

fn load_certs(path: &str) -> Result<Vec<tokio_rustls::rustls::pki_types::CertificateDer<'static>>> {
    let mut reader = BufReader::new(File::open(path).with_context(|| format!("open cert {path}"))?);
    let certs = rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("parse cert {path}"))?;
    if certs.is_empty() {
        anyhow::bail!("cert file {path} contains no CERTIFICATE block");
    }
    Ok(certs)
}

fn load_private_key(path: &str) -> Result<PrivateKeyDer<'static>> {
    let mut reader = BufReader::new(File::open(path).with_context(|| format!("open key {path}"))?);
    rustls_pemfile::private_key(&mut reader)
        .with_context(|| format!("parse key {path}"))?
        .ok_or_else(|| anyhow::anyhow!("key file {path} contains no private key block"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_go_fingerprint_format_case_insensitive_with_colons() {
        let fp = parse_fingerprint(
            "SHA256:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF",
        )
        .unwrap();

        assert_eq!(
            hex::encode(fp),
            "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"
        );
    }
}
