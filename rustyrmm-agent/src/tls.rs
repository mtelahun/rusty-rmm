//! TLS to the Agent port. At Pairing the Agent knows only the CA's SHA-256 fingerprint (from the
//! install one-liner), so it trusts the CA certificate in the Server's chain whose fingerprint
//! matches, and remembers it. After Pairing, that saved CA is the only trust root.

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct PinnedCaVerifier {
    fingerprint: Vec<u8>,
    provider: Arc<CryptoProvider>,
    /// The CA certificate (DER) that matched the pin, once a handshake succeeded.
    pub matched_ca: Mutex<Option<Vec<u8>>>,
}

impl PinnedCaVerifier {
    /// `fingerprint_hex`: SHA-256 of the CA certificate's DER, hex (colons and case ignored).
    pub fn new(fingerprint_hex: &str) -> anyhow::Result<Self> {
        let cleaned: String = fingerprint_hex
            .chars()
            .filter(|c| *c != ':')
            .collect::<String>()
            .to_lowercase();
        let fingerprint = hex::decode(&cleaned)?;
        anyhow::ensure!(
            fingerprint.len() == 32,
            "CA fingerprint must be a SHA-256 (64 hex characters)"
        );
        Ok(Self {
            fingerprint,
            provider: Arc::new(rustls::crypto::ring::default_provider()),
            matched_ca: Mutex::new(None),
        })
    }
}

impl ServerCertVerifier for PinnedCaVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let ca = intermediates
            .iter()
            .find(|c| Sha256::digest(c.as_ref()).as_slice() == self.fingerprint)
            .ok_or_else(|| {
                Error::General(
                    "no certificate in the Server's chain matches the pinned CA fingerprint".into(),
                )
            })?;
        // Full path validation (signature, validity, name) with only the pinned CA trusted.
        let mut roots = RootCertStore::empty();
        roots
            .add(ca.clone().into_owned())
            .map_err(|e| Error::General(format!("pinned CA is not a valid trust anchor: {e}")))?;
        let verified =
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), self.provider.clone())
                .build()
                .map_err(|e| Error::General(e.to_string()))?
                .verify_server_cert(end_entity, &[], server_name, ocsp_response, now)?;
        *self.matched_ca.lock().unwrap() = Some(ca.to_vec());
        Ok(verified)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_accepts_colons_and_rejects_wrong_length() {
        let hex64 = "AB".repeat(32);
        let colons = hex64
            .as_bytes()
            .chunks(2)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join(":");
        assert!(PinnedCaVerifier::new(&hex64).is_ok());
        assert!(PinnedCaVerifier::new(&colons).is_ok());
        assert!(PinnedCaVerifier::new("abcd").is_err());
        assert!(PinnedCaVerifier::new("not hex").is_err());
    }
}
