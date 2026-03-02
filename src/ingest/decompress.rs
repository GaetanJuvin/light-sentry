use axum::http::HeaderMap;
use flate2::read::{GzDecoder, ZlibDecoder};
use std::io::Read;

/// Decompress the body based on Content-Encoding header.
/// Sentry SDKs send gzip or deflate compressed payloads.
pub fn decompress_body(headers: &HeaderMap, raw: &[u8]) -> Result<String, String> {
    let encoding = headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let bytes = match encoding {
        "gzip" => {
            let mut decoder = GzDecoder::new(raw);
            let mut buf = Vec::new();
            decoder
                .read_to_end(&mut buf)
                .map_err(|e| format!("gzip decode error: {e}"))?;
            buf
        }
        "deflate" => {
            let mut decoder = ZlibDecoder::new(raw);
            let mut buf = Vec::new();
            decoder
                .read_to_end(&mut buf)
                .map_err(|e| format!("deflate decode error: {e}"))?;
            buf
        }
        _ => {
            // Try gzip anyway — some SDKs don't set the header
            if raw.starts_with(&[0x1f, 0x8b]) {
                let mut decoder = GzDecoder::new(raw);
                let mut buf = Vec::new();
                if decoder.read_to_end(&mut buf).is_ok() {
                    buf
                } else {
                    raw.to_vec()
                }
            } else {
                raw.to_vec()
            }
        }
    };

    String::from_utf8(bytes).map_err(|e| format!("invalid UTF-8: {e}"))
}
