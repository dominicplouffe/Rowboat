//! TLS configuration shared by the drivers (rustls with the `ring` provider).
//!
//! Maps PostgreSQL's `sslmode` levels onto certificate verification:
//!
//! | mode                 | encrypts | checks the chain | checks the host name |
//! | -------------------- | -------- | ---------------- | -------------------- |
//! | `prefer` / `require` | yes      | no               | no                   |
//! | `verify-ca`          | yes      | yes              | no                   |
//! | `verify-full`        | yes      | yes              | yes                  |

use std::path::Path;
use std::sync::{Arc, Once};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};

use crate::connections::{Tls, TlsMode};
use crate::db::{DbError, DbResult};

/// Installs `ring` as the process-wide crypto provider. Idempotent.
pub fn ensure_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn read_pem(path: &str, what: &str) -> DbResult<Vec<u8>> {
    std::fs::read(Path::new(path))
        .map_err(|e| DbError::connection(format!("Could not read the {what} ({path}): {e}")))
}

pub fn certificates(path: &str, what: &str) -> DbResult<Vec<CertificateDer<'static>>> {
    let pem = read_pem(path, what)?;
    let certs: Vec<_> = rustls_pemfile::certs(&mut pem.as_slice())
        .collect::<Result<_, _>>()
        .map_err(|e| DbError::connection(format!("The {what} ({path}) is not valid PEM: {e}")))?;
    if certs.is_empty() {
        return Err(DbError::connection(format!(
            "The {what} ({path}) contains no certificates."
        )));
    }
    Ok(certs)
}

fn private_key(path: &str) -> DbResult<PrivateKeyDer<'static>> {
    let pem = read_pem(path, "client key")?;
    rustls_pemfile::private_key(&mut pem.as_slice())
        .map_err(|e| DbError::connection(format!("The client key ({path}) is not valid PEM: {e}")))?
        .ok_or_else(|| DbError::connection(format!("The client key ({path}) contains no key.")))
}

/// Trust anchors: the given CA file, or the operating system's store.
fn roots(tls: &Tls) -> DbResult<RootCertStore> {
    let mut store = RootCertStore::empty();
    if tls.ca_cert_path.is_empty() {
        let native = rustls_native_certs::load_native_certs();
        store.add_parsable_certificates(native.certs);
    } else {
        for cert in certificates(&tls.ca_cert_path, "CA certificate")? {
            store.add(cert).map_err(|e| {
                DbError::connection(format!("The CA certificate is not usable: {e}"))
            })?;
        }
    }
    Ok(store)
}

/// Accepts any certificate: encryption without authentication, which is what
/// `sslmode=require` means in PostgreSQL.
#[derive(Debug)]
struct AcceptAny(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
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
}

/// Verifies the certificate chain but not the host name (`verify-ca`).
#[derive(Debug)]
struct ChainOnly(Arc<WebPkiServerVerifier>);

impl ServerCertVerifier for ChainOnly {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self
            .0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
        {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

/// A rustls client configuration for the given TLS settings. Call only for
/// modes other than `Disable`.
pub fn client_config(tls: &Tls) -> DbResult<ClientConfig> {
    ensure_provider();
    let builder = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| DbError::connection(format!("TLS setup failed: {e}")))?;

    let builder = match tls.mode {
        TlsMode::Disable | TlsMode::Prefer | TlsMode::Require => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny(provider()))),
        TlsMode::VerifyCa | TlsMode::VerifyFull => {
            let roots = roots(tls)?;
            if roots.is_empty() {
                return Err(DbError::connection(
                    "No trusted CA certificates are available to verify the server.",
                )
                .with_hint("Choose a CA certificate file in the connection's TLS settings."));
            }
            if tls.mode == TlsMode::VerifyFull {
                builder.with_root_certificates(roots)
            } else {
                let verifier =
                    WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider())
                        .build()
                        .map_err(|e| DbError::connection(format!("TLS setup failed: {e}")))?;
                builder
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(ChainOnly(verifier)))
            }
        }
    };

    if tls.client_cert_path.is_empty() {
        Ok(builder.with_no_client_auth())
    } else {
        if tls.client_key_path.is_empty() {
            return Err(DbError::connection(
                "A client certificate needs its private key file too.",
            ));
        }
        builder
            .with_client_auth_cert(
                certificates(&tls.client_cert_path, "client certificate")?,
                private_key(&tls.client_key_path)?,
            )
            .map_err(|e| DbError::connection(format!("The client certificate is not usable: {e}")))
    }
}
