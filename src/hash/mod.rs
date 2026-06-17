const B62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

pub(crate) fn full_scope() -> Vec<String> {
    vec![".".to_string()]
}

pub(crate) fn expectation_id(question: &str, expected_answer: &str, instructions: &str) -> String {
    // Expectation IDs are 20-character base62 IDs derived from the question,
    // expected answer, and a deterministic hash of the resolved expectation
    // instructions.
    let instructions_hash = hash_60(instructions.as_bytes());
    let mut input = Vec::new();
    push_expectation_id_frame(&mut input, "question", question.as_bytes());
    push_expectation_id_frame(&mut input, "expectedAnswer", expected_answer.as_bytes());
    push_expectation_id_frame(&mut input, "instructionsHash", instructions_hash.as_bytes());
    expectation_id_base62_20(&input)
}

pub(crate) fn hash_60(input: &[u8]) -> String {
    let hash = fnv64_with_seed(FNV_OFFSET, input);
    encode_60_bits(hash & ((1u64 << 60) - 1))
}

fn expectation_id_base62_20(input: &[u8]) -> String {
    let first = fnv64_with_seed(FNV_OFFSET, input);
    let second = fnv64_with_seed(FNV_OFFSET ^ 0x9e37_79b9_7f4a_7c15, input);
    let value = (((first & 0x7fff_ffff_ffff_ffff) as u128) << 56) | ((second >> 8) as u128);
    encode_base62_20(value)
}

fn push_expectation_id_frame(output: &mut Vec<u8>, name: &str, value: &[u8]) {
    output.extend_from_slice(name.as_bytes());
    output.push(0);
    output.extend_from_slice(value.len().to_string().as_bytes());
    output.push(0);
    output.extend_from_slice(value);
    output.push(0);
}

pub(crate) fn fnv64_with_seed(seed: u64, input: &[u8]) -> u64 {
    let mut hash = seed;
    for byte in input {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
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

#[cfg(test)]
mod tests {
    use super::expectation_id;

    #[test]
    fn expectation_id_changes_when_expected_answer_changes() {
        let yes = expectation_id("Does it pass?", "yes", "");
        let no = expectation_id("Does it pass?", "no", "");

        assert_ne!(yes, no);
    }
}
