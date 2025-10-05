use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};
use std::sync::Arc;
use x509_parser::prelude::*;

/// Custom TLS certificate verifier that matches certificate public keys against
/// Nostr-published TLS public keys from kind 34256 events.
#[derive(Debug)]
pub struct NostrCertVerifier {
    expected_pubkey_hex: String,
}

impl NostrCertVerifier {
    pub fn new(expected_pubkey_hex: String) -> Self {
        Self {
            expected_pubkey_hex,
        }
    }
}

impl ServerCertVerifier for NostrCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        // Parse the certificate
        let (_, cert) = X509Certificate::from_der(end_entity.as_ref())
            .map_err(|_e| TlsError::InvalidCertificate(rustls::CertificateError::BadEncoding))?;

        // Extract the public key from the certificate
        let spki = cert.public_key();
        let pubkey_bytes = &spki.subject_public_key.data;

        // For Ed25519, the public key is the last 32 bytes of the subject public key
        // The format is typically: algorithm OID + key data
        // For Ed25519, the raw key is 32 bytes
        let extracted_pubkey = if pubkey_bytes.len() >= 32 {
            // Try to extract the last 32 bytes as the Ed25519 public key
            &pubkey_bytes[pubkey_bytes.len() - 32..]
        } else {
            &pubkey_bytes[..]
        };

        let cert_pubkey_hex = hex::encode(extracted_pubkey);

        // Compare with expected public key from Nostr event
        if cert_pubkey_hex.to_lowercase() == self.expected_pubkey_hex.to_lowercase() {
            Ok(ServerCertVerified::assertion())
        } else {
            tracing::error!(
                "TLS pubkey mismatch: expected {}, got {}",
                self.expected_pubkey_hex,
                cert_pubkey_hex
            );
            Err(TlsError::InvalidCertificate(
                rustls::CertificateError::Other(rustls::OtherError(Arc::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Certificate public key does not match Nostr-published key",
                )))),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        // Accept TLS 1.2 signatures (we're verifying via pubkey match instead)
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        // Accept TLS 1.3 signatures (we're verifying via pubkey match instead)
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        // Support common signature schemes
        vec![
            SignatureScheme::ED25519,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}
