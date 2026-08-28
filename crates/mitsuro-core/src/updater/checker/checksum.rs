use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};

pub(crate) const MAX_CHECKSUM_BYTES: usize = 1024;

pub(crate) fn parse_published_sha256(body: &[u8], expected_archive_name: &str) -> Result<[u8; 32]> {
    if body.is_empty() {
        return Err(anyhow!("Release checksum file is empty"));
    }
    if body.len() > MAX_CHECKSUM_BYTES {
        return Err(anyhow!(
            "Release checksum file exceeds {} bytes",
            MAX_CHECKSUM_BYTES
        ));
    }
    if expected_archive_name.is_empty()
        || !expected_archive_name.is_ascii()
        || expected_archive_name.contains(['\r', '\n'])
        || expected_archive_name.contains('/')
        || expected_archive_name.contains('\\')
    {
        return Err(anyhow!("Invalid expected archive name"));
    }

    let text = std::str::from_utf8(body).context("Release checksum file is not UTF-8")?;
    if !text.is_ascii() {
        return Err(anyhow!("Release checksum file must contain only ASCII"));
    }
    let record = text.strip_suffix('\n').unwrap_or(text);
    if record.contains(['\r', '\n']) {
        return Err(anyhow!(
            "Release checksum file must contain exactly one record"
        ));
    }
    if record.len() < 66 {
        return Err(anyhow!("Release checksum record is malformed"));
    }

    let (hex_digest, file_field) = record.split_at(64);
    let published_name = file_field
        .strip_prefix("  ")
        .ok_or_else(|| anyhow!("Release checksum record must use sha256sum format"))?;
    if published_name != expected_archive_name {
        return Err(anyhow!(
            "Release checksum names '{}', expected '{}'",
            published_name,
            expected_archive_name
        ));
    }

    let mut digest = [0_u8; 32];
    for (index, pair) in hex_digest.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let high = decode_hex_nibble(pair[0])
            .ok_or_else(|| anyhow!("Release checksum digest is not valid hexadecimal"))?;
        let low = decode_hex_nibble(pair[1])
            .ok_or_else(|| anyhow!("Release checksum digest is not valid hexadecimal"))?;
        digest[index] = (high << 4) | low;
    }

    Ok(digest)
}

fn decode_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn verify_archive_sha256(archive: &[u8], expected: &[u8; 32]) -> Result<()> {
    let actual = Sha256::digest(archive);
    if actual[..] != expected[..] {
        return Err(anyhow!(
            "Release archive checksum verification failed; refusing to extract"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARCHIVE_NAME: &str = "mitsuro-x86_64-unknown-linux-gnu.tar.gz";
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn parses_exact_sha256sum_record() {
        let body = format!("{}  {}\n", ABC_SHA256, ARCHIVE_NAME);
        let parsed = parse_published_sha256(body.as_bytes(), ARCHIVE_NAME).expect("valid record");
        verify_archive_sha256(b"abc", &parsed).expect("matching archive");
    }
}
