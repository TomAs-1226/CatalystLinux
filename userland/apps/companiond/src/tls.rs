//! TLS 1.3 with the handheld's own certificate: self-signed, made the first
//! time a computer comes to pair (and again after every computer has been
//! unpaired), and pinned by each computer when it pairs.

use std::sync::{Arc, Mutex};

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::ServerConfig;

use crate::store::{self, Paths};

/// The certificate and key, made if they are not there yet.
fn certificate(paths: &Paths) -> Result<(Vec<u8>, Vec<u8>), String> {
    if let (Ok(cert), Ok(key)) = (std::fs::read(paths.cert()), std::fs::read(paths.key())) {
        if !cert.is_empty() && !key.is_empty() {
            return Ok((cert, key));
        }
    }
    let made = rcgen::generate_simple_self_signed(vec![format!("{}.local", store::name()), "catalyst.local".into()]).map_err(|e| format!("could not make a certificate: {e}"))?;
    let (cert, key) = (made.cert.der().to_vec(), made.signing_key.serialize_der());
    store::write_private(&paths.key(), &key).map_err(|e| format!("could not keep the key: {e}"))?;
    store::write_private(&paths.cert(), &cert).map_err(|e| format!("could not keep the certificate: {e}"))?;
    Ok((cert, key))
}

/// The server's TLS, made again only when the certificate changes.
pub struct Tls {
    made: Mutex<Option<(Vec<u8>, Arc<ServerConfig>)>>,
}

impl Tls {
    pub fn new() -> Self {
        Tls { made: Mutex::new(None) }
    }

    /// The configuration for a connection, and its certificate's fingerprint.
    pub fn config(&self, paths: &Paths) -> Result<(Arc<ServerConfig>, [u8; 32]), String> {
        let (cert, key) = certificate(paths)?;
        let fingerprint = catalyst_companion_proto::fingerprint(&cert);
        let mut made = self.made.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((was, config)) = made.as_ref() {
            if *was == cert {
                return Ok((config.clone(), fingerprint));
            }
        }
        let mut provider = rustls::crypto::ring::default_provider();
        // ChaCha20-Poly1305 first: the CM0's A53 has no AES instructions.
        provider.cipher_suites = vec![rustls::crypto::ring::cipher_suite::TLS13_CHACHA20_POLY1305_SHA256, rustls::crypto::ring::cipher_suite::TLS13_AES_128_GCM_SHA256];
        let mut config = ServerConfig::builder_with_provider(Arc::new(provider))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| e.to_string())?
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(cert.clone())], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)))
            .map_err(|e| format!("the certificate will not do: {e}"))?;
        config.ignore_client_order = true;
        let config = Arc::new(config);
        *made = Some((cert, config.clone()));
        Ok((config, fingerprint))
    }
}
