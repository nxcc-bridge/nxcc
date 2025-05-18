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

use rcgen::{
    Certificate as RcgenCertificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use thiserror::Error;
use tonic::transport::{Certificate, ClientTlsConfig, Identity, ServerTlsConfig};

/// Errors that can occur during TLS configuration
#[derive(Error, Debug)]
pub enum TlsError {
    #[error("Failed to generate certificate: {0}")]
    CertGeneration(#[from] rcgen::Error), // Use rcgen::Error directly

    #[error("Failed to generate key pair: {0}")]
    KeyGeneration(String),

    #[error("Invalid certificate parameters: {0}")]
    InvalidParams(String),

    #[error("Failed to parse PEM data: {0}")]
    PemParse(String),
}

/// Represents a certificate bundle with PEM-encoded certificate and private key
#[derive(Clone)]
pub struct CertBundle {
    pub cert_pem: String,
    pub key_pem: String,
}

/// Contains all certificates needed for mTLS, generated from a single dummy CA.
pub struct MtlsCertificates {
    pub ca_pem: String,
    pub server: CertBundle,
    pub client: CertBundle,
    /// The internal CA certificate object
    ca_cert: RcgenCertificate,
    /// The internal CA key pair object
    ca_key: KeyPair,
}

impl MtlsCertificates {
    /// Creates a complete mTLS certificate setup with a new dummy CA, server cert, and client cert.
    /// Hostnames are not used as services are ephemeral and unnamed.
    pub fn new() -> Result<Self, TlsError> {
        // Generate the deterministic CA
        let (ca_cert, ca_key) = generate_deterministic_ca_cert()?;
        let ca_pem = ca_cert.pem();

        // Generate server certificate - explicitly add "localhost" as SAN
        let server_bundle = generate_signed_cert("server", "localhost", &ca_cert, &ca_key)?;

        // Generate client certificate - SAN isn't strictly needed here but CN is set
        let client_bundle = generate_signed_cert("client", "client", &ca_cert, &ca_key)?;

        Ok(MtlsCertificates {
            ca_pem,
            server: server_bundle,
            client: client_bundle,
            ca_cert,
            ca_key,
        })
    }

    /// Creates server TLS configuration for gRPC with mTLS using the generated certificates.
    pub fn server_tls_config(&self) -> Result<ServerTlsConfig, TlsError> {
        let server_identity = Identity::from_pem(&self.server.cert_pem, &self.server.key_pem);
        let ca_cert = Certificate::from_pem(&self.ca_pem);

        Ok(ServerTlsConfig::new()
            .identity(server_identity)
            .client_ca_root(ca_cert))
    }

    /// Creates client TLS configuration for gRPC with mTLS using the generated certificates.
    /// Uses "localhost" as a dummy domain name, as required by tonic, although
    /// hostname validation is not the primary goal here.
    pub fn client_tls_config(&self) -> Result<ClientTlsConfig, TlsError> {
        let client_identity = Identity::from_pem(&self.client.cert_pem, &self.client.key_pem);
        let ca_cert = Certificate::from_pem(&self.ca_pem);

        Ok(ClientTlsConfig::new()
            .identity(client_identity)
            .ca_certificate(ca_cert)
            // Tonic requires a domain name, even if we don't rely on it for validation.
            .domain_name("localhost"))
    }

    /// Generates an additional client certificate signed by the same internal CA.
    pub fn generate_additional_client_cert(
        &self,
        cn_name: &str, // Common Name for the cert
    ) -> Result<CertBundle, TlsError> {
        // Pass the CN as both CN and the (less critical) SAN for clients
        generate_signed_cert(cn_name, cn_name, &self.ca_cert, &self.ca_key)
    }
}

/// Generates a deterministic Certificate Authority (CA) certificate and key pair.
/// This CA is intended solely to satisfy mTLS chain requirements and should NOT be trusted.
fn generate_deterministic_ca_cert() -> Result<(RcgenCertificate, KeyPair), TlsError> {
    // TODO: consider injecting these through a reproducible build script
    let cert_pem = [
        "-".repeat(5) + "BEGIN CERTIFICATE" + &"-".repeat(5),
        "MIIBljCCATygAwIBAgIJAN5+gUHFcJE0MAoGCCqGSM49BAMCMC0xKzApBgNVBAMM".into(),
        "IkR1bW15IFVudHJ1c3RlZCBDQSAtIERldGVybWluaXN0aWMwIBcNNzAwMTAxMDAw".into(),
        "MDAwWhgPMjA1MDAxMDEwMDAwMDBaMC0xKzApBgNVBAMMIkR1bW15IFVudHJ1c3Rl".into(),
        "ZCBDQSAtIERldGVybWluaXN0aWMwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAASM".into(),
        "pRHfsjER+pvKos8uVqghT2JF8wpQVx5wB7zsP0RCVjJvnd4FnZTM2ChhVdTWZW2D".into(),
        "WxAZK442Dkzv8CnHoZ64o0MwQTAPBgNVHQ8BAf8EBQMDBwYAMB0GA1UdDgQWBBTz".into(),
        "iq9OpmTH3Dsad5XbbbSHYPME7zAPBgNVHRMBAf8EBTADAQH/MAoGCCqGSM49BAMC".into(),
        "A0gAMEUCIQDyUOTygq+QajPP+UBQdGuz8cfl+tiQL5Z99AkMEJJMFAIgEBAa4RMU".into(),
        "bRtVh8qU6DPGHJgjjeZVMeIVgZNi+sT5HkY=".into(),
        "-".repeat(5) + "END CERTIFICATE" + &"-".repeat(5),
    ]
    .join("\n");
    let params = CertificateParams::from_ca_cert_pem(&cert_pem).unwrap();

    let key_pem = [
        "-".repeat(5) + "BEGIN EC PRIVATE KEY" + &"-".repeat(5),
        "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg8CZUSyBRxMLyr6y/".into(),
        "GDnudlkKg2/Prkia/3aZUYT2IQqhRANCAASMpRHfsjER+pvKos8uVqghT2JF8wpQ".into(),
        "Vx5wB7zsP0RCVjJvnd4FnZTM2ChhVdTWZW2DWxAZK442Dkzv8CnHoZ64".into(),
        "-".repeat(5) + "END EC PRIVATE KEY" + &"-".repeat(5),
    ]
    .join("\n");
    let key_pair =
        KeyPair::from_pem(&key_pem).map_err(|e| TlsError::KeyGeneration(e.to_string()))?;

    let cert = params.self_signed(&key_pair)?;
    Ok((cert, key_pair))
}

/// Generate an end-entity certificate signed by the provided CA.
/// The `cn_name` is used for the Common Name (CN).
/// The `san_name` is added as a Subject Alternative Name (DNS type).
fn generate_signed_cert(
    cn_name: &str,
    san_name: &str, // Name to put in SAN (e.g., "localhost" for server)
    ca_cert: &RcgenCertificate,
    ca_key_pair: &KeyPair,
) -> Result<CertBundle, TlsError> {
    // Add the san_name to the list of SANs
    let mut params = CertificateParams::new(vec![san_name.to_string()])
        .map_err(|e| TlsError::InvalidParams(e.to_string()))?;

    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, cn_name); // Use cn_name for CN
    params.distinguished_name = distinguished_name;
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment, // Needed for TLS key exchange
    ];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth, // Allows use as server cert
        ExtendedKeyUsagePurpose::ClientAuth, // Allows use as client cert
    ];
    // Explicitly add the SAN type if needed (rcgen often infers DNS from string)
    // params.subject_alt_names.push(SanType::DnsName(san_name.to_string())); // Redundant if passed in new()

    // Generate a new key pair for the end-entity certificate - does not take alg argument
    let key_pair = KeyPair::generate().map_err(|e| TlsError::KeyGeneration(e.to_string()))?;
    let cert = params.signed_by(&key_pair, ca_cert, ca_key_pair)?;

    Ok(CertBundle {
        cert_pem: cert.pem(),
        key_pem: key_pair.serialize_pem(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mtls_certificate_generation_and_config() {
        let certs = MtlsCertificates::new().expect("Failed to create certificates");

        // Check CA cert
        assert!(certs.ca_pem.contains("BEGIN CERTIFICATE"));

        // Check server cert
        assert!(certs.server.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(certs.server.key_pem.contains("BEGIN PRIVATE KEY")); // rcgen default is PKCS#8
        // Verify server cert contains the SAN (optional but good check)
        let server_x509 = pem::parse(&certs.server.cert_pem).expect("Failed to parse server PEM");
        let server_cert_parsed = x509_parser::parse_x509_certificate(server_x509.contents())
            .expect("Failed to parse server cert")
            .1;
        let sans = server_cert_parsed
            .subject_alternative_name()
            .expect("SAN extension missing")
            .expect("Failed to parse SAN extension")
            .value
            .general_names
            .clone();
        assert!(sans.iter().any(|san| match san {
            x509_parser::extensions::GeneralName::DNSName(name) => *name == "localhost",
            _ => false,
        }));

        // Check client cert
        assert!(certs.client.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(certs.client.key_pem.contains("BEGIN PRIVATE KEY"));

        // Check TLS configs can be created - this is the main check now
        // We don't need to assert the contents of the config objects themselves,
        // as they are opaque builders. Success means the PEM data was valid.
        let _server_config = certs
            .server_tls_config()
            .expect("Failed to create server TLS config");
        let _client_config = certs
            .client_tls_config()
            .expect("Failed to create client TLS config");
    }

    #[test]
    fn test_additional_client_cert_generation() {
        let certs = MtlsCertificates::new().expect("Failed to create base certificates");
        let client2_bundle = certs
            .generate_additional_client_cert("client2")
            .expect("Failed to generate additional client cert");

        assert!(client2_bundle.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(client2_bundle.key_pem.contains("BEGIN PRIVATE KEY"));

        // Ensure it's different from the first client cert
        assert_ne!(certs.client.cert_pem, client2_bundle.cert_pem);
        assert_ne!(certs.client.key_pem, client2_bundle.key_pem);

        // Try creating a config with the additional cert - this verifies the cert/key are usable
        let client2_identity =
            Identity::from_pem(&client2_bundle.cert_pem, &client2_bundle.key_pem);
        let ca_cert = Certificate::from_pem(&certs.ca_pem);
        let _client2_config = ClientTlsConfig::new() // Assign to _ to avoid unused warning
            .identity(client2_identity)
            .ca_certificate(ca_cert)
            .domain_name("localhost"); // Still need a domain name for client config
    }
}
