use std::{io, sync::Arc, time::Duration};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme};
use thiserror::Error;
use x509_parser::prelude::{FromDer, X509Certificate};

use crate::nns::{PublishedTlsKey, TlsAlgorithm};

#[derive(Debug, Error)]
pub enum SecureTransportError {
    #[error("failed to build http client: {0}")]
    HttpClient(#[from] reqwest::Error),
}

#[derive(Clone)]
pub struct SecureHttpClient {
    client: reqwest::Client,
}

impl SecureHttpClient {
    pub fn new(tls_key: Option<&PublishedTlsKey>) -> Result<Self, SecureTransportError> {
        let client = if let Some(key) = tls_key {
            let config = build_pinned_config(key);
            reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .use_preconfigured_tls(config)
                .build()?
        } else {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()?
        };
        Ok(SecureHttpClient { client })
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

fn build_pinned_config(tls_key: &PublishedTlsKey) -> ClientConfig {
    let verifier = Arc::new(PublishedKeyVerifier::new(tls_key.clone()));
    ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth()
}

#[derive(Debug)]
struct PublishedKeyVerifier {
    key: PublishedTlsKey,
}

impl PublishedKeyVerifier {
    fn new(key: PublishedTlsKey) -> Self {
        Self { key }
    }
}

impl ServerCertVerifier for PublishedKeyVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let (_, cert) = X509Certificate::from_der(end_entity.as_ref())
            .map_err(|_| TlsError::InvalidCertificate(rustls::CertificateError::BadEncoding))?;

        if !algorithm_matches(&cert, &self.key) {
            return Err(build_mismatch_error("certificate algorithm mismatch"));
        }

        if !spki_matches(&cert, &self.key) {
            return Err(build_mismatch_error(
                "certificate subject public key mismatch",
            ));
        }

        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
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

fn algorithm_matches(cert: &X509Certificate<'_>, key: &PublishedTlsKey) -> bool {
    match key.algorithm {
        TlsAlgorithm::Ed25519 => {
            cert.public_key().algorithm.algorithm.to_id_string() == "1.3.101.112"
        }
    }
}

fn spki_matches(cert: &X509Certificate<'_>, key: &PublishedTlsKey) -> bool {
    cert.public_key().subject_public_key.data == key.spki.as_slice()
}

fn build_mismatch_error(message: &str) -> TlsError {
    let io_error = io::Error::new(io::ErrorKind::PermissionDenied, message.to_string());
    TlsError::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
        Arc::new(io_error),
    )))
}
