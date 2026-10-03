//! Verifies a TLS certificate chain against a bundled set of trusted
//! roots, entirely in memory, for platforms where `git2::opts`'s
//! `set_ssl_cert_file`/`set_ssl_cert_dir` cannot be used at all.
//!
//! That was the original plan for Android (vendor a CA bundle, point
//! libgit2 at its path -- see this repository's git history for the
//! first attempt), and it cannot work there: `openssl-src` (which builds the
//! OpenSSL that `libgit2-sys` links against) passes `no-stdio` to OpenSSL's
//! `Configure` specifically for Android targets, which compiles out
//! OpenSSL's file-backed BIO -- the thing `SSL_CTX_load_verify_locations`
//! (behind both of those `git2::opts` functions) needs to open a cert file
//! or directory from disk. Confirmed by testing that call against a real
//! certificate, a garbage file, and a nonexistent path in the same run: all
//! three fail identically, which only makes sense if the mechanism itself
//! -- not the file's content -- is what's broken.
//!
//! `no-stdio`'s own rationale looks obsolete (it dates to a 2018 GCC-era
//! NDK linker failure; the NDK has been Clang-only since 2018), and
//! there's an open upstream issue with the maintainer receptive to
//! dropping it entirely:
//! <https://github.com/alexcrichton/openssl-src-rs/issues/287>. If that
//! lands, the original file-based approach would start working too --
//! but this module would still be worth keeping: it doesn't depend on
//! Android having a system CA store at all (it still won't, even without
//! `no-stdio`), and it doesn't need `openssl-src` to be rebuilt/bumped
//! first.
//!
//! [`CertificateVerifier`](crate::CertificateVerifier) sidesteps this by
//! never asking libgit2 to load anything from disk: `git2`'s own
//! `certificate_check` callback fires instead (libgit2's transport layer
//! calls it whenever a certificate check callback is registered at all,
//! regardless of whether its own OS-level verification found anything to
//! check against -- see `check_certificate` in libgit2's
//! `transports/httpclient.c`), and this module verifies the certificate
//! itself using the `openssl` crate's in-memory APIs, which `no-stdio`
//! doesn't touch.
//!
//! The one complication: `certificate_check` only ever hands over the
//! leaf certificate the server presented, never any intermediates (it's
//! populated from `SSL_get_peer_certificate`, not
//! `SSL_get_peer_cert_chain`). A normal CA-issued leaf chains through at
//! least one intermediate before reaching a trusted root, so this fetches
//! the missing intermediate itself over plain HTTP from the leaf's
//! Authority Information Access "CA Issuers" URL -- the same "AIA chasing"
//! a browser does when a server's handshake doesn't include its own
//! intermediate. The trust decision is still made by verifying the
//! resulting chain up to the bundled root store; a wrong or hostile AIA
//! response can only ever fail verification, never itself become trusted.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use openssl::nid::Nid;
use openssl::stack::Stack;
use openssl::x509::store::{X509Store, X509StoreBuilder};
use openssl::x509::{X509, X509StoreContext};

use crate::CertificateVerifier;

/// However many missing-intermediate hops to chase before giving up. A
/// real chain is one or two deep; this is generous headroom, not an
/// expected case.
const MAX_AIA_HOPS: u32 = 4;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Verifies certificates against a fixed set of trusted roots given as a
/// PEM bundle (Android: `immermemo/assets/cacert.pem`, curl's own extract
/// of Mozilla's root store).
pub struct ChainVerifier {
    roots: X509Store,
}

impl ChainVerifier {
    pub fn new(root_bundle_pem: &[u8]) -> Result<Self> {
        let mut builder = X509StoreBuilder::new()?;
        for cert in X509::stack_from_pem(root_bundle_pem)? {
            builder.add_cert(cert)?;
        }
        Ok(Self {
            roots: builder.build(),
        })
    }
}

impl CertificateVerifier for ChainVerifier {
    fn trust(&self, host: &str, leaf_der: &[u8]) -> bool {
        match verify(&self.roots, leaf_der) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("certificate for {host} not trusted: {e:#}");
                false
            }
        }
    }
}

fn verify(roots: &X509Store, leaf_der: &[u8]) -> Result<()> {
    let leaf = X509::from_der(leaf_der).context("parse the server's leaf certificate")?;
    let mut chain: Stack<X509> = Stack::new()?;

    if chain_is_trusted(roots, &leaf, &chain)? {
        return Ok(());
    }

    let mut current = leaf.clone();
    for _ in 0..MAX_AIA_HOPS {
        let Some(url) = ca_issuers_url(&current) else {
            break;
        };
        let der =
            fetch_der_over_http(&url).with_context(|| format!("fetching issuer from {url}"))?;
        let issuer = X509::from_der(&der).context("parse the fetched intermediate certificate")?;
        chain.push(issuer.clone())?;
        if chain_is_trusted(roots, &leaf, &chain)? {
            return Ok(());
        }
        current = issuer;
    }

    bail!("could not build a certificate chain to a trusted root")
}

fn chain_is_trusted(roots: &X509Store, leaf: &X509, chain: &Stack<X509>) -> Result<bool> {
    let mut ctx = X509StoreContext::new()?;
    Ok(ctx.init(roots, leaf, chain, |c| c.verify_cert())?)
}

/// The "CA Issuers" URL from the certificate's Authority Information
/// Access extension, if it has one and it's plain HTTP.
fn ca_issuers_url(cert: &X509) -> Option<String> {
    cert.authority_info()?.iter().find_map(|access| {
        if access.method().nid() != Nid::AD_CA_ISSUERS {
            return None;
        }
        let uri = access.location().uri()?;
        uri.starts_with("http://").then(|| uri.to_owned())
    })
}

/// A minimal HTTP/1.1 GET, plain (not HTTPS -- fetching the one thing that
/// would let us verify a TLS connection over a TLS connection of its own
/// just moves the bootstrapping problem rather than solving it). CA
/// issuer endpoints are conventionally served over plain HTTP for exactly
/// this reason.
fn fetch_der_over_http(url: &str) -> Result<Vec<u8>> {
    let rest = url.strip_prefix("http://").context("not an http:// URL")?;
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (rest, "/".to_owned()),
    };
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => (h, p.parse().context("invalid port in AIA URL")?),
        None => (authority, 80u16),
    };

    let mut stream = TcpStream::connect((host, port))?;
    stream.set_read_timeout(Some(FETCH_TIMEOUT))?;
    stream.set_write_timeout(Some(FETCH_TIMEOUT))?;
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: */*\r\n\r\n");
    stream.write_all(request.as_bytes())?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;

    let header_end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("malformed HTTP response: no header terminator")?;
    let (header_bytes, rest) = response.split_at(header_end);
    let body = &rest[4..];
    let headers =
        std::str::from_utf8(header_bytes).context("HTTP response headers were not UTF-8")?;

    let status_line = headers.lines().next().context("empty HTTP response")?;
    if !status_line
        .split_whitespace()
        .nth(1)
        .is_some_and(|c| c == "200")
    {
        bail!("AIA fetch failed: {status_line}");
    }
    if headers
        .lines()
        .any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:"))
    {
        bail!("chunked AIA responses are not supported");
    }

    Ok(body.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
    use std::path::Path;

    /// Reuses the app's vendored root bundle so this proves the exact
    /// bytes that ship in the APK, not a second copy kept in sync by hand.
    fn root_bundle() -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../immermemo/assets/cacert.pem");
        std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"))
    }

    /// Connects for real and pulls the leaf certificate a live host
    /// actually presents, so the test exercises the real AIA-chasing path
    /// rather than a synthetic fixture. Verification of that connection is
    /// deliberately switched off here (`SslVerifyMode::NONE`) -- this is
    /// only how the test *obtains* a real leaf certificate to hand to
    /// [`verify`], which is the thing actually under test.
    fn fetch_leaf_der(host: &str) -> Vec<u8> {
        let mut connector = SslConnector::builder(SslMethod::tls()).unwrap();
        connector.set_verify(SslVerifyMode::NONE);
        let connector = connector.build();
        let stream = TcpStream::connect((host, 443)).unwrap();
        let stream = connector.connect(host, stream).unwrap();
        stream
            .ssl()
            .peer_certificate()
            .expect("server presented a certificate")
            .to_der()
            .unwrap()
    }

    #[test]
    fn a_real_lets_encrypt_leaf_chains_through_a_fetched_intermediate() {
        // Any host whose handshake doesn't include its own intermediate
        // (i.e. the ordinary case for `certificate_check`, which never
        // sees the handshake's intermediates at all) exercises the AIA
        // fetch. tailscale.com is issued by Let's Encrypt, which is
        // exactly the CA this whole module exists for.
        let leaf = fetch_leaf_der("tailscale.com");
        let roots = ChainVerifier::new(&root_bundle()).unwrap().roots;
        assert!(
            chain_is_trusted(
                &roots,
                &X509::from_der(&leaf).unwrap(),
                &Stack::new().unwrap()
            )
            .map(|trusted_without_chasing| !trusted_without_chasing)
            .unwrap_or(true),
            "test is meaningless if the leaf alone already verifies"
        );
        assert!(verify(&roots, &leaf).is_ok());
    }

    #[test]
    fn an_unrelated_self_signed_certificate_is_rejected() {
        use openssl::asn1::Asn1Time;
        use openssl::hash::MessageDigest;
        use openssl::pkey::PKey;
        use openssl::rsa::Rsa;
        use openssl::x509::X509Name;

        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let mut name = X509Name::builder().unwrap();
        name.append_entry_by_text("CN", "not-a-real-ca.example")
            .unwrap();
        let name = name.build();
        let mut builder = openssl::x509::X509Builder::new().unwrap();
        builder.set_version(2).unwrap();
        builder.set_subject_name(&name).unwrap();
        builder.set_issuer_name(&name).unwrap();
        builder.set_pubkey(&key).unwrap();
        builder
            .set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        builder
            .set_not_after(&Asn1Time::days_from_now(1).unwrap())
            .unwrap();
        builder.sign(&key, MessageDigest::sha256()).unwrap();
        let cert = builder.build();

        let verifier = ChainVerifier::new(&root_bundle()).unwrap();
        assert!(!verifier.trust("not-a-real-ca.example", &cert.to_der().unwrap()));
    }
}
