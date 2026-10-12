//! Encrypted token storage blob encoding and decoding for cross-device transfer.
//!
//! Format: `[iv length: 1 byte][iv][ciphertext, including GCM tag]`.

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn encode(iv: &[u8], ciphertext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + iv.len() + ciphertext.len());
    out.push(u8::try_from(iv.len()).expect("a GCM IV is well under 256 bytes"));
    out.extend_from_slice(iv);
    out.extend_from_slice(ciphertext);
    out
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn decode(blob: &[u8]) -> anyhow::Result<(&[u8], &[u8])> {
    let (&iv_len, rest) = blob
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("empty token file"))?;
    anyhow::ensure!(rest.len() >= iv_len as usize, "truncated token file");
    Ok(rest.split_at(iv_len as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let blob = encode(&[1, 2, 3, 4], b"ciphertext");
        let (iv, ciphertext) = decode(&blob).unwrap();
        assert_eq!(iv, [1, 2, 3, 4]);
        assert_eq!(ciphertext, b"ciphertext");
    }

    #[test]
    fn rejects_a_truncated_or_empty_blob() {
        assert!(decode(&[5, 1, 2]).is_err());
        assert!(decode(&[]).is_err());
    }
}
