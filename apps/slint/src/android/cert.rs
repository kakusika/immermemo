//! Wires up TLS certificate verification for sync on Android, using
//! `immermemo_sync::tls::ChainVerifier` against the vendored Mozilla root
//! bundle -- see that module's doc for why Android needs this at all
//! instead of `git2::opts::set_ssl_cert_file`/`_dir` (which cannot work
//! here: Android's vendored OpenSSL build has file-based certificate
//! loading compiled out).
//!
//! The bundle itself is curl's own extract of Mozilla's root store
//! (<https://curl.se/docs/caextract.html>); there is no source for it on
//! the device itself to read instead, so it goes stale over time and needs
//! re-fetching from that URL and re-vendoring by hand.

use immermemo_sync::CertificateVerifier;
use immermemo_sync::tls::ChainVerifier;

const ROOT_BUNDLE: &[u8] = include_bytes!("../../assets/cacert.pem");

pub fn verifier() -> anyhow::Result<Box<dyn CertificateVerifier>> {
    Ok(Box::new(ChainVerifier::new(ROOT_BUNDLE)?))
}
