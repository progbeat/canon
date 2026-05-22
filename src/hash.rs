use crate::{B64_URL, FNV_OFFSET, FNV_PRIME};

const B62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

pub(crate) fn full_scope() -> Vec<String> {
    vec![".".to_string()]
}

pub(crate) fn expectation_id(prompt: &str, expected: &str) -> String {
    // Expectation IDs are the Cache spec's 20-character base62 IDs. The
    // base64url `hash_120` helper below is for other cache keys, not IDs.
    expectation_id_base62_20(format!("q\0{}\0a\0{}", prompt, expected).as_bytes())
}

pub(crate) fn hash_key(key: &str) -> String {
    hash_60(key.as_bytes())
}

pub(crate) fn hash_60(input: &[u8]) -> String {
    let hash = fnv64_with_seed(FNV_OFFSET, input);
    encode_60_bits(hash & ((1u64 << 60) - 1))
}

pub(crate) fn hash_120(input: &[u8]) -> String {
    let first = fnv64_with_seed(FNV_OFFSET, input);
    let second = fnv64_with_seed(FNV_OFFSET ^ 0x9e37_79b9_7f4a_7c15, input);
    let mut bytes = [0u8; 15];
    bytes[..8].copy_from_slice(&first.to_be_bytes());
    bytes[8..].copy_from_slice(&second.to_be_bytes()[..7]);
    encode_base64url_no_pad(&bytes)
}

fn expectation_id_base62_20(input: &[u8]) -> String {
    let first = fnv64_with_seed(FNV_OFFSET, input);
    let second = fnv64_with_seed(FNV_OFFSET ^ 0x9e37_79b9_7f4a_7c15, input);
    let value = (((first & 0x7fff_ffff_ffff_ffff) as u128) << 56) | ((second >> 8) as u128);
    encode_base62_20(value)
}

pub(crate) fn fnv64_with_seed(seed: u64, input: &[u8]) -> u64 {
    let mut hash = seed;
    for byte in input {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

pub(crate) fn encode_base64url_no_pad(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() * 4).div_ceil(3));
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        let value = ((a as u32) << 16) | ((b as u32) << 8) | c as u32;
        out.push(B64_URL[((value >> 18) & 0x3f) as usize] as char);
        out.push(B64_URL[((value >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_URL[((value >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64_URL[(value & 0x3f) as usize] as char);
        }
    }
    out
}

pub(crate) fn encode_60_bits(value: u64) -> String {
    let mut out = String::with_capacity(10);
    for shift in (0..60).step_by(6).rev() {
        let index = ((value >> shift) & 0x3f) as usize;
        out.push(B64_URL[index] as char);
    }
    out
}

fn encode_base62_20(mut value: u128) -> String {
    let mut bytes = [B62[0]; 20];
    for byte in bytes.iter_mut().rev() {
        *byte = B62[(value % 62) as usize];
        value /= 62;
    }
    String::from_utf8(bytes.to_vec()).expect("base62 alphabet is valid UTF-8")
}
