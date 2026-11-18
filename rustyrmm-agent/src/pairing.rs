//! `rustyrmm-agent pair`: exchange a Pairing Token for an Agent certificate.

use std::path::Path;
use std::sync::Arc;

use rcgen::{CertificateParams, KeyPair};
use rustyrmm_proto::agent::v1::agent_service_client::AgentServiceClient;
use rustyrmm_proto::agent::v1::PairRequest;
use tonic::transport::{Channel, ClientTlsConfig};

use crate::state::{Config, Paired};
use crate::tls::PinnedCaVerifier;

/// Pairs with the Server at `server` (`host:port`), trusting only a CA whose SHA-256 matches
/// `ca_fingerprint`. Saves the key (never sent anywhere), certificate and CA to `dir`.
/// Returns the new Endpoint id.
pub async fn pair(
    dir: &Path,
    server: &str,
    pairing_token: &str,
    ca_fingerprint: &str,
) -> anyhow::Result<String> {
    let verifier = Arc::new(PinnedCaVerifier::new(ca_fingerprint)?);
    let channel = Channel::from_shared(format!("https://{server}"))?
        .tls_config_with_verifier(
            ClientTlsConfig::new().domain_name(host(server)?),
            verifier.clone(),
        )?
        .connect()
        .await?;

    let key = KeyPair::generate()?;
    let csr = CertificateParams::new(Vec::<String>::new())?
        .serialize_request(&key)?
        .pem()?;
    let resp = AgentServiceClient::new(channel)
        .pair(PairRequest {
            pairing_token: pairing_token.to_owned(),
            csr_pem: csr,
        })
        .await?
        .into_inner();

    let ca_der = verifier
        .matched_ca
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| anyhow::anyhow!("TLS handshake did not record the pinned CA"))?;
    Paired {
        dir: dir.to_path_buf(),
        config: Config {
            server: server.to_owned(),
            endpoint_id: resp.endpoint_id.clone(),
        },
        key_pem: key.serialize_pem(),
        cert_pem: resp.certificate_pem,
        ca_pem: pem::encode(&pem::Pem::new("CERTIFICATE", ca_der)),
    }
    .save()?;
    Ok(resp.endpoint_id)
}

/// The host part of `host:port`, used as the TLS server name.
pub fn host(server: &str) -> anyhow::Result<String> {
    let uri: tonic::codegen::http::Uri = format!("https://{server}").parse()?;
    uri.host()
        .map(|h| h.trim_matches(['[', ']']).to_owned())
        .ok_or_else(|| anyhow::anyhow!("invalid Server address {server:?}; expected host:port"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn host_strips_port_and_ipv6_brackets() {
        assert_eq!(
            super::host("rmm.example.com:7443").unwrap(),
            "rmm.example.com"
        );
        assert_eq!(super::host("[::1]:7443").unwrap(), "::1");
    }
}
