//! The Server's private CA: signs the Agent port's certificate and Agent client certificates.

use std::fs;
use std::io::Write;
use std::path::Path;

use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, SerialNumber,
};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const CA_VALIDITY: Duration = Duration::days(3650);
const LEAF_VALIDITY: Duration = Duration::days(365);
/// Backdate certificates slightly so small clock skew on Endpoints doesn't reject them.
const CLOCK_SKEW: Duration = Duration::minutes(5);

pub struct Ca {
    issuer: Issuer<'static, KeyPair>,
    cert_pem: String,
    cert_der: Vec<u8>,
}

pub struct IssuedCert {
    pub pem: String,
    pub serial: Vec<u8>,
    pub not_before: OffsetDateTime,
    pub not_after: OffsetDateTime,
}

impl Ca {
    /// Loads `ca.crt` and `ca.key` from `dir`, generating them on first start.
    pub fn load_or_create(dir: &Path) -> anyhow::Result<Self> {
        let (cert_path, key_path) = (dir.join("ca.crt"), dir.join("ca.key"));
        if !key_path.exists() {
            fs::create_dir_all(dir)?;
            let key = KeyPair::generate()?;
            let mut params = CertificateParams::new(Vec::<String>::new())?;
            params
                .distinguished_name
                .push(DnType::CommonName, "RustyRMM Agent CA");
            params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
            params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
            let now = OffsetDateTime::now_utc();
            params.not_before = now - CLOCK_SKEW;
            params.not_after = now + CA_VALIDITY;
            let cert = params.self_signed(&key)?;
            write_private(&key_path, key.serialize_pem().as_bytes())?;
            fs::write(&cert_path, cert.pem())?;
        }
        let key = KeyPair::from_pem(&fs::read_to_string(&key_path)?)?;
        let cert_pem = fs::read_to_string(&cert_path)?;
        let cert_der = pem::parse(&cert_pem)?.into_contents();
        let issuer = Issuer::from_ca_cert_pem(&cert_pem, key)?;
        Ok(Self {
            issuer,
            cert_pem,
            cert_der,
        })
    }

    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    /// Hex SHA-256 of the CA certificate (DER), which Agents pin at Pairing.
    pub fn fingerprint_sha256(&self) -> String {
        hex::encode(Sha256::digest(&self.cert_der))
    }

    /// Issues the Agent port's TLS identity for `names`. Returns (certificate chain PEM, key PEM);
    /// the chain includes the CA so Agents can check it against their pinned fingerprint.
    pub fn agent_port_identity(&self, names: Vec<String>) -> anyhow::Result<(String, String)> {
        let key = KeyPair::generate()?;
        let mut params = CertificateParams::new(names)?;
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        set_leaf_validity(&mut params);
        let cert = params.signed_by(&key, &self.issuer)?;
        Ok((
            format!("{}{}", cert.pem(), self.cert_pem),
            key.serialize_pem(),
        ))
    }

    /// Signs an Agent's CSR. Only the CSR's public key is used (its signature proves possession
    /// of the private key); subject, usage and validity are set here, with the Endpoint id as CN.
    pub fn sign_agent_csr(&self, csr_pem: &str, endpoint_id: Uuid) -> anyhow::Result<IssuedCert> {
        let csr = CertificateSigningRequestParams::from_pem(csr_pem)?;
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params
            .distinguished_name
            .push(DnType::CommonName, endpoint_id.to_string());
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let serial = new_serial();
        params.serial_number = Some(SerialNumber::from_slice(&serial));
        let (not_before, not_after) = set_leaf_validity(&mut params);
        let signed = CertificateSigningRequestParams {
            params,
            public_key: csr.public_key,
        }
        .signed_by(&self.issuer)?;
        Ok(IssuedCert {
            pem: signed.pem(),
            serial,
            not_before,
            not_after,
        })
    }
}

fn set_leaf_validity(params: &mut CertificateParams) -> (OffsetDateTime, OffsetDateTime) {
    let now = OffsetDateTime::now_utc();
    params.not_before = now - CLOCK_SKEW;
    params.not_after = now + LEAF_VALIDITY;
    (params.not_before, params.not_after)
}

/// 16 random bytes with the top bit cleared, so the DER integer stays positive.
fn new_serial() -> Vec<u8> {
    let mut serial: [u8; 16] = rand::random();
    serial[0] &= 0x7f;
    serial.to_vec()
}

/// Writes a file readable only by its owner (the CA key must be root-only).
fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)?.write_all(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_once_then_reloads_the_same_ca() {
        let dir = tempfile::tempdir().unwrap();
        let first = Ca::load_or_create(dir.path()).unwrap();
        let second = Ca::load_or_create(dir.path()).unwrap();
        assert_eq!(first.fingerprint_sha256(), second.fingerprint_sha256());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join("ca.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn agent_cert_names_the_endpoint_and_ignores_csr_subject() {
        let dir = tempfile::tempdir().unwrap();
        let ca = Ca::load_or_create(dir.path()).unwrap();
        let agent_key = KeyPair::generate().unwrap();
        let mut csr_params = CertificateParams::new(vec!["evil.example".to_string()]).unwrap();
        csr_params
            .distinguished_name
            .push(DnType::CommonName, "someone-else");
        let csr = csr_params
            .serialize_request(&agent_key)
            .unwrap()
            .pem()
            .unwrap();

        let id = Uuid::now_v7();
        let issued = ca.sign_agent_csr(&csr, id).unwrap();
        let der = pem::parse(&issued.pem).unwrap().into_contents();
        let (_, cert) = x509_parser::parse_x509_certificate(&der).unwrap();
        let cn = cert.subject().iter_common_name().next().unwrap();
        assert_eq!(cn.as_str().unwrap(), id.to_string());
        assert!(cert.subject_alternative_name().unwrap().is_none());
        assert_eq!(cert.raw_serial(), issued.serial.as_slice());
    }
}
