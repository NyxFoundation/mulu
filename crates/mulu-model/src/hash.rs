use sha2::{Digest, Sha256};

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Canonical JSON: `serde_json::Value` serialises maps with sorted keys
/// (the default `Map` is a `BTreeMap`) and no whitespace.
pub fn canonical_json(v: &serde_json::Value) -> String {
    serde_json::to_string(v).expect("serialising a Value cannot fail")
}
