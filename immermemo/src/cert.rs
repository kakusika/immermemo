//! TLS certificate verification for sync on mobile platforms (Android, iOS),
//! using `immermemo_sync::tls::ChainVerifier` against the vendored Mozilla
//! root bundle.
//!
//! Mobile targets with vendored OpenSSL cannot rely on host file-based certificate
//! loading (`/etc/ssl/certs/...`). The bundle is curl's extract of Mozilla's
//! root store (<https://curl.se/docs/caextract.html>).

use immermemo_sync::CertificateVerifier;
use immermemo_sync::tls::ChainVerifier;

const ROOT_BUNDLE: &[u8] = include_bytes!("../assets/cacert.pem");

pub fn verifier() -> anyhow::Result<Box<dyn CertificateVerifier>> {
    Ok(Box::new(ChainVerifier::new(ROOT_BUNDLE)?))
}
