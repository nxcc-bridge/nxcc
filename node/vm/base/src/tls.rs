//! TLS Configuration Utilities using a Self-Signed Dummy CA.
//!
//! This module provides functions to generate a temporary, self-signed Certificate Authority (CA)
//! and use it to issue end-entity certificates for both the server and clients.
//!
//! **Purpose of the Dummy CA:**
//! The primary goal here is **not** to establish trust in the traditional PKI sense. Instead,
//! this dummy CA serves a specific technical purpose: enabling mutual TLS (mTLS) authentication
//! in scenarios where a pre-existing, trusted CA infrastructure is unavailable or undesirable.
//! Tonic's mTLS implementation requires *some* CA certificate for validating the peer's certificate chain.
//! By generating a unique CA for each server instance and ensuring both the server and its intended client(s)
//! use certificates signed by *this specific CA*, we achieve:
//!   1. **Channel Encryption:** Standard TLS benefit.
//!   2. **Mutual Authentication:** Both server and client *must* present a certificate signed by this ephemeral CA.
//!      This prevents connections from clients/servers that don't possess a cert from this specific CA instance.
//!   3. **Client Certificate Extraction:** Enables the server to reliably extract the client's certificate for further logic (like client binding).
//!
//! **Security Considerations:**
//! - This CA is **untrusted** and should **never** be installed in system trust stores.
//! - The security relies on the CA's private key remaining confidential to the server process that generated it
//!   and the client binding logic, not on the CA's identity itself.
//! - Authorization decisions should **not** be based solely on the fact that a certificate was validated against this dummy CA.
use std::error::Error;

use rcgen::{
    BasicConstraints, Certificate as RcgenCertificate, CertificateParams, DistinguishedName,
    DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use tonic::transport::{Certificate, ClientTlsConfig, Identity, ServerTlsConfig};

/// Generates a deterministic Certificate Authority (CA) certificate and key pair.
/// This CA is intended solely to satisfy mTLS chain requirements and should NOT be trusted.
/// It's deterministic *within a single run* but not across different process executions unless the keypair is saved/seeded.
pub fn generate_ca_cert() -> Result<(RcgenCertificate, KeyPair), Box<dyn Error + Send + Sync>> {
    let mut params = CertificateParams::default();
    let mut distinguished_name = DistinguishedName::new();
    // Use a fixed, recognizable name for the dummy CA
    distinguished_name.push(DnType::CommonName, "Dummy Untrusted CA");
    params.distinguished_name = distinguished_name;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];

    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;
    Ok((cert, key_pair))
}

/// Generate an end-entity certificate signed by the provided CA.
pub fn generate_signed_cert(
    common_name: &str,
    ca_cert: &RcgenCertificate,
    ca_key_pair: &KeyPair,
) -> Result<(String, String), Box<dyn Error + Send + Sync>> {
    let mut params = CertificateParams::new(vec![common_name.to_string()])?;
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, common_name);
    params.distinguished_name = distinguished_name;
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth, // Allow use as client cert too if needed
    ];

    let key_pair = KeyPair::generate()?;
    let cert = params.signed_by(&key_pair, ca_cert, ca_key_pair)?;

    Ok((cert.pem(), key_pair.serialize_pem()))
}

/// Create server TLS configuration for gRPC with mTLS using a dummy CA.
/// Requires client certificates signed by the provided dummy CA.
pub fn create_server_tls_config(
    server_cert_pem: String,
    server_key_pem: String,
    dummy_ca_cert_pem: String,
) -> Result<ServerTlsConfig, Box<dyn Error + Send + Sync>> {
    let server_identity = Identity::from_pem(server_cert_pem, server_key_pem);
    let dummy_ca_cert = Certificate::from_pem(dummy_ca_cert_pem);

    // Configure TLS:
    // - Use the server's identity (signed by the dummy CA)
    // - Require a client certificate and validate it against the dummy CA.
    //   This satisfies mTLS requirements but doesn't imply trust in the CA.
    let server_tls_config = ServerTlsConfig::new()
        .identity(server_identity)
        .client_ca_root(dummy_ca_cert); // Requires client cert signed by this CA

    Ok(server_tls_config)
}

/// Create client TLS configuration for connecting to the server using a dummy CA.
pub fn create_client_tls_config(
    client_cert_pem: String,
    client_key_pem: String,
    dummy_ca_cert_pem: String,
    domain_name: &str,
) -> Result<ClientTlsConfig, Box<dyn Error + Send + Sync>> {
    let client_identity = Identity::from_pem(client_cert_pem, client_key_pem);
    let dummy_ca_cert = Certificate::from_pem(dummy_ca_cert_pem);

    // Configure TLS:
    // - Use the client's identity (signed by the dummy CA)
    // - Use the dummy CA to validate the server's certificate chain.
    //   This satisfies mTLS requirements but doesn't imply trust in the CA.
    let client_tls_config = ClientTlsConfig::new()
        .identity(client_identity)
        .ca_certificate(dummy_ca_cert) // Validate server cert against the dummy CA
        .domain_name(domain_name.to_string());

    Ok(client_tls_config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ca_generation() {
        let result = generate_ca_cert();
        assert!(result.is_ok());
        let (cert, _key) = result.unwrap();
        assert!(cert.pem().contains("BEGIN CERTIFICATE"));
        assert!(matches!(cert.params().is_ca, IsCa::Ca(_)));
    }

    #[test]
    fn test_signed_certificate_generation() {
        let (ca_cert, ca_key) = generate_ca_cert().unwrap();
        let result = generate_signed_cert("test.example.com", &ca_cert, &ca_key);
        assert!(result.is_ok());

        let (cert_pem, key_pem) = result.unwrap();
        assert!(cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(key_pem.contains("BEGIN PRIVATE KEY"));

        // Basic check: ensure it's not marked as CA
        let params = rcgen::CertificateParams::from_ca_cert_pem(&cert_pem).unwrap();
        assert!(!matches!(params.is_ca, IsCa::Ca(_)));
    }

    #[test]
    fn test_server_tls_config() {
        let (ca_cert, ca_key) = generate_ca_cert().unwrap();
        let (server_cert_pem, server_key_pem) =
            generate_signed_cert("server.example.com", &ca_cert, &ca_key).unwrap();

        let result = create_server_tls_config(server_cert_pem, server_key_pem, ca_cert.pem());
        assert!(result.is_ok());
    }

    #[test]
    fn test_client_tls_config() {
        let (ca_cert, ca_key) = generate_ca_cert().unwrap();
        let (client_cert_pem, client_key_pem) =
            generate_signed_cert("client.example.com", &ca_cert, &ca_key).unwrap();

        let result = create_client_tls_config(
            client_cert_pem,
            client_key_pem,
            ca_cert.pem(),
            "server.example.com",
        );
        assert!(result.is_ok());
    }
}
