//! Deterministic canonical JSON used by Centralized write signatures.
//!
//! Rules (wire format, must be reproduced byte-for-byte by the server):
//!
//! - object keys are sorted recursively in ascending Unicode code-point order;
//! - arrays keep their order;
//! - compact separators (`,` and `:`), UTF-8, no insignificant whitespace;
//! - floating-point numbers are rejected (integer / string / bool / null only);
//! - the top-level value may be any JSON value (bodies are objects in practice).
//!
//! The signature payload is `op || "\n" || canonical_json(body)`.

use serde::Serialize;
use serde_json::Value;

use crate::ChainSdkError;

/// Serialize a value as canonical JSON bytes.
///
/// This is the same algorithm used by `Centralized` request signing; it is public
/// so servers and other SDKs can reproduce signatures exactly.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ChainSdkError> {
    let value = serde_json::to_value(value)
        .map_err(|error| ChainSdkError::InvalidArgument(format!("canonical JSON: {error}")))?;
    let canonical = canonicalize(value)?;
    serde_json::to_vec(&canonical)
        .map_err(|error| ChainSdkError::InvalidArgument(format!("canonical JSON: {error}")))
}

fn canonicalize(value: Value) -> Result<Value, ChainSdkError> {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut sorted = serde_json::Map::new();
            for (key, value) in entries {
                sorted.insert(key, canonicalize(value)?);
            }
            Ok(Value::Object(sorted))
        }
        Value::Array(items) => {
            let mut canonical = Vec::with_capacity(items.len());
            for item in items {
                canonical.push(canonicalize(item)?);
            }
            Ok(Value::Array(canonical))
        }
        Value::Number(number) if number.is_f64() => Err(ChainSdkError::InvalidArgument(
            "canonical JSON rejects floating point numbers".into(),
        )),
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::canonical_json;

    #[test]
    fn sorts_keys_recursively() {
        let bytes = canonical_json(&serde_json::json!({
            "b": 1,
            "a": { "d": 4, "c": 3 }
        }))
        .unwrap();
        assert_eq!(bytes, br#"{"a":{"c":3,"d":4},"b":1}"#);
    }

    #[test]
    fn preserves_array_order() {
        let bytes = canonical_json(&serde_json::json!({ "items": [3, 1, 2] })).unwrap();
        assert_eq!(bytes, br#"{"items":[3,1,2]}"#);
    }

    #[test]
    fn rejects_floats() {
        assert!(canonical_json(&serde_json::json!({ "x": 1.5 })).is_err());
    }

    #[test]
    fn handles_unicode_and_escapes() {
        let bytes = canonical_json(&serde_json::json!({ "name": "张三", "quote": "\"" })).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "{\"name\":\"张三\",\"quote\":\"\\\"\"}"
        );
    }

    #[test]
    fn handles_empty_containers() {
        assert_eq!(canonical_json(&serde_json::json!({})).unwrap(), b"{}");
        assert_eq!(canonical_json(&serde_json::json!([])).unwrap(), b"[]");
    }
}
