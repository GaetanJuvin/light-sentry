use sha2::{Digest, Sha256};
use serde_json::Value;

pub fn compute_fingerprint(event: &Value) -> String {
    let mut hasher = Sha256::new();

    // Try exception-based fingerprint first
    if let Some(values) = event.pointer("/exception/values").and_then(|v| v.as_array()) {
        if let Some(exc) = values.last() {
            let exc_type = exc.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let exc_value = exc.get("value").and_then(|v| v.as_str()).unwrap_or("");
            hasher.update(exc_type.as_bytes());
            hasher.update(b":");
            hasher.update(exc_value.as_bytes());

            // Find top in_app frame
            if let Some(frames) = exc.pointer("/stacktrace/frames").and_then(|v| v.as_array()) {
                for frame in frames.iter().rev() {
                    let in_app = frame
                        .get("in_app")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if in_app {
                        let filename =
                            frame.get("filename").and_then(|v| v.as_str()).unwrap_or("");
                        let function =
                            frame.get("function").and_then(|v| v.as_str()).unwrap_or("");
                        hasher.update(b":");
                        hasher.update(filename.as_bytes());
                        hasher.update(b":");
                        hasher.update(function.as_bytes());
                        break;
                    }
                }
            }

            return hex::encode(hasher.finalize());
        }
    }

    // Fallback: hash the message
    let message = event
        .get("message")
        .and_then(|v| v.as_str())
        .or_else(|| {
            event
                .get("logentry")
                .and_then(|v| v.get("message"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("unknown");
    hasher.update(message.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fingerprint_from_exception() {
        let event = json!({
            "exception": {
                "values": [{
                    "type": "ValueError",
                    "value": "invalid literal",
                    "stacktrace": {
                        "frames": [
                            {"filename": "lib.py", "function": "inner", "in_app": false},
                            {"filename": "app.py", "function": "handler", "in_app": true}
                        ]
                    }
                }]
            }
        });
        let fp = compute_fingerprint(&event);
        assert!(!fp.is_empty());
        let fp2 = compute_fingerprint(&event);
        assert_eq!(fp, fp2);
    }

    #[test]
    fn fingerprint_without_exception_uses_message() {
        let event = json!({"message": "something broke", "level": "error"});
        let fp = compute_fingerprint(&event);
        assert!(!fp.is_empty());
    }

    #[test]
    fn different_exceptions_different_fingerprints() {
        let e1 = json!({"exception": {"values": [{"type": "TypeError", "value": "a"}]}});
        let e2 = json!({"exception": {"values": [{"type": "ValueError", "value": "b"}]}});
        assert_ne!(compute_fingerprint(&e1), compute_fingerprint(&e2));
    }
}
