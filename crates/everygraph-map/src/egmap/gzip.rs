// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Gzip, the one compression of the file (§ 1): one member, nothing after it, and never
//! more than a cap decoded.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::bufread::GzDecoder;
use flate2::write::GzEncoder;

use super::{EgmapError, malformed};

/// Level 6: zlib's default. The same bytes give the same output for one version of the
/// codec, so a build is reproducible; a new codec version may change the bytes, not
/// what they decode to.
pub(super) fn compress(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(6));
    encoder
        .write_all(data)
        .expect("writing into a Vec does not fail");
    encoder.finish().expect("writing into a Vec does not fail")
}

/// The bytes `data` decodes to, refused when they pass `cap`, when `data` is not one
/// whole gzip member, or when bytes follow the member.
pub(super) fn decompress(data: &[u8], cap: usize, what: &str) -> Result<Vec<u8>, EgmapError> {
    let mut decoder = GzDecoder::new(data);
    let mut out = Vec::new();
    let limit = u64::try_from(cap).map_or(u64::MAX, |c| c.saturating_add(1));
    decoder
        .by_ref()
        .take(limit)
        .read_to_end(&mut out)
        .map_err(|e| malformed(format!("{what}: not a gzip member ({e})")))?;
    if out.len() > cap {
        return Err(malformed(format!(
            "{what} decodes to more than {cap} bytes"
        )));
    }
    if !decoder.into_inner().is_empty() {
        return Err(malformed(format!("{what}: bytes follow the gzip member")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompression_stops_at_the_cap_and_refuses_trailing_bytes() {
        let data: Vec<u8> = std::iter::repeat_n(7, 5000).collect();
        let packed = compress(&data);
        assert_eq!(decompress(&packed, 5000, "t").unwrap(), data);
        assert!(decompress(&packed, 4999, "t").is_err());
        let mut twice = packed.clone();
        twice.extend_from_slice(&packed);
        assert!(decompress(&twice, 1 << 20, "t").is_err());
        assert!(decompress(&packed[..packed.len() - 1], 1 << 20, "t").is_err());
    }
}
