//! Charset-robust decoding of subtitle files. The Python parser assumed UTF-8;
//! donghua/anime `.ass` rips are frequently GBK/Big5/UTF-16, so we detect the
//! encoding (honoring a BOM, else chardetng) and decode with encoding_rs.

use std::path::Path;

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::Encoding;

use crate::error::AppResult;

/// Decode raw subtitle bytes to a `String`. Honors a UTF-8/UTF-16 BOM if present;
/// otherwise sniffs the encoding with chardetng. Malformed sequences are replaced
/// (never errors), so a best-effort string always comes back.
pub fn decode_bytes(bytes: &[u8]) -> String {
    let encoding = match Encoding::for_bom(bytes) {
        Some((enc, _bom_len)) => enc,
        None => {
            // Subtitles are local files, not script-running Web content, so
            // ISO-2022-JP stays a candidate (the pre-1.0 chardetng default).
            let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
            detector.feed(bytes, true);
            detector.guess(None, Utf8Detection::Allow)
        }
    };
    // `decode` re-sniffs and strips a leading BOM that matches `encoding`.
    let (text, _enc, _had_errors) = encoding.decode(bytes);
    text.into_owned()
}

/// Read and decode an `.ass` file from disk.
pub fn decode_file(path: &Path) -> AppResult<String> {
    let bytes = std::fs::read(path)?;
    Ok(decode_bytes(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_plain_utf8() {
        assert_eq!(decode_bytes("Hello 世界".as_bytes()), "Hello 世界");
    }

    #[test]
    fn strips_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("Hello".as_bytes());
        assert_eq!(decode_bytes(&bytes), "Hello");
    }

    #[test]
    fn decodes_utf16le_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for u in "Hi 世界".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_bytes(&bytes), "Hi 世界");
    }

    #[test]
    fn decodes_gbk_chinese() {
        // A long-enough Chinese sample so chardetng reliably picks GBK/GB18030.
        let sample = "修仙者突破了金丹期，灵气充沛，准备渡劫飞升。武林盟主召集江湖各大门派，讨论轻功和内力修炼之道。";
        let (bytes, _, _) = encoding_rs::GBK.encode(sample);
        assert_eq!(decode_bytes(&bytes), sample);
    }
}
