use super::super::program::command_output_token;
use crate::output::command_output_trimmed;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

const HEX: &[u8; 16] = b"0123456789abcdef";
pub(super) const RAW_PATH_HEX_PREFIX: &str = "\0raw-path-hex:";

#[derive(Clone, Copy)]
pub(super) enum GitObjectHashAlgorithm {
    Sha1,
    Sha256,
}

pub(crate) fn git_object_oid_has_known_shape(object_id: &str) -> bool {
    [GitObjectHashAlgorithm::Sha1, GitObjectHashAlgorithm::Sha256]
        .into_iter()
        .any(|algorithm| git_object_oid_has_hex_len(object_id, git_object_oid_hex_len(algorithm)))
}

pub(crate) fn git_object_oid_has_hex_len(object_id: &str, hex_len: usize) -> bool {
    object_id.len() == hex_len
        && object_id
            .as_bytes()
            .iter()
            .all(|byte| hex_nibble(*byte).is_ok())
}

pub(super) fn git_object_oid_hex_len(algorithm: GitObjectHashAlgorithm) -> usize {
    match algorithm {
        GitObjectHashAlgorithm::Sha1 => 40,
        GitObjectHashAlgorithm::Sha256 => 64,
    }
}

pub(super) fn visible_tree_oid_from_entries(
    entries: &[String],
    object_hash_algorithm: GitObjectHashAlgorithm,
) -> Result<String, String> {
    // `visibleTreeOid` is a Git-compatible tree object ID. We rebuild the scoped
    // tree from Git-reported modes/object IDs, then hash the canonical `tree
    // <len>\0<body>` bytes with the repository's object hash algorithm.
    //
    // The entries are leaf paths from `git ls-files --stage` or `git ls-tree
    // -r` without `-t`. Directory nodes are always synthesized from the scoped
    // leaves, so excluded descendants cannot affect the resulting tree OID.
    let mut tree = TreeNode::default();
    for entry in entries {
        let parsed = parse_visible_tree_entry(entry)?;
        tree.insert(&parsed.path, parsed.mode, parsed.object_id)?;
    }
    tree.oid(object_hash_algorithm)
}

#[derive(Default)]
struct TreeNode {
    entries: BTreeMap<Vec<u8>, TreeEntry>,
}

enum TreeEntry {
    File { mode: String, object_id: String },
    Directory(TreeNode),
}

pub(super) struct VisibleTreeEntry {
    pub(super) mode: String,
    pub(super) object_id: String,
    pub(super) path: Vec<Vec<u8>>,
}

impl TreeNode {
    fn insert(&mut self, path: &[Vec<u8>], mode: String, object_id: String) -> Result<(), String> {
        let Some((name, rest)) = path.split_first() else {
            return Err("visible tree entry path must not be empty".to_string());
        };
        if rest.is_empty() {
            if is_git_tree_mode(&mode) {
                return Err("visible tree entries must be leaf paths".to_string());
            }
            self.entries
                .insert(name.clone(), TreeEntry::File { mode, object_id });
            return Ok(());
        }
        let entry = self
            .entries
            .entry(name.clone())
            .or_insert_with(|| TreeEntry::Directory(TreeNode::default()));
        match entry {
            TreeEntry::Directory(directory) => directory.insert(rest, mode, object_id),
            TreeEntry::File { .. } => Err(format!(
                "visible tree path conflicts with file: {}",
                String::from_utf8_lossy(name)
            )),
        }
    }

    fn oid(&self, object_hash_algorithm: GitObjectHashAlgorithm) -> Result<String, String> {
        let mut entries = self
            .entries
            .iter()
            .map(|(name, entry)| entry.encoded(name, object_hash_algorithm))
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by(git_tree_entry_cmp);
        let mut body = Vec::new();
        for entry in entries {
            body.extend_from_slice(entry.mode.as_bytes());
            body.push(b' ');
            body.extend_from_slice(&entry.name);
            body.push(0);
            body.extend_from_slice(&entry.object_id);
        }
        git_object_id(object_hash_algorithm, "tree", &body)
    }
}

impl TreeEntry {
    fn encoded(
        &self,
        name: &[u8],
        object_hash_algorithm: GitObjectHashAlgorithm,
    ) -> Result<EncodedTreeEntry, String> {
        match self {
            TreeEntry::File { mode, object_id } => Ok(EncodedTreeEntry {
                name: name.to_vec(),
                mode: mode.clone(),
                object_id: hex_object_id_bytes(object_id)?,
                is_directory: false,
            }),
            TreeEntry::Directory(directory) => Ok(EncodedTreeEntry {
                name: name.to_vec(),
                mode: "40000".to_string(),
                object_id: hex_object_id_bytes(&directory.oid(object_hash_algorithm)?)?,
                is_directory: true,
            }),
        }
    }
}

struct EncodedTreeEntry {
    name: Vec<u8>,
    mode: String,
    object_id: Vec<u8>,
    is_directory: bool,
}

pub(super) fn parse_visible_tree_entry(entry: &str) -> Result<VisibleTreeEntry, String> {
    let (metadata, path) = entry
        .split_once('\t')
        .ok_or_else(|| "visible tree entry missing path".to_string())?;
    let mut fields = metadata.split_whitespace();
    let mode = fields
        .next()
        .ok_or_else(|| format!("visible tree entry missing mode for {}", path))?;
    let object_id = fields
        .next()
        .ok_or_else(|| format!("visible tree entry missing object id for {}", path))?;
    if let Some(stage) = fields.next() {
        if stage != "0" {
            return Err(format!(
                "visible tree entry has unresolved stage for {}",
                path
            ));
        }
    }
    let path = visible_tree_path_components(path)?;
    Ok(VisibleTreeEntry {
        mode: mode.to_string(),
        object_id: object_id.to_string(),
        path,
    })
}

fn visible_tree_path_components(path: &str) -> Result<Vec<Vec<u8>>, String> {
    let path = if let Some(encoded) = path.strip_prefix(RAW_PATH_HEX_PREFIX) {
        raw_path_hex_bytes(encoded)?
    } else {
        if path.contains('\0') {
            return Err("visible tree entry contains invalid NUL path marker".to_string());
        }
        path.as_bytes().to_vec()
    };
    Ok(path
        .split(|byte| *byte == b'/')
        .filter(|component| !component.is_empty())
        .map(|component| component.to_vec())
        .collect())
}

fn raw_path_hex_bytes(encoded: &str) -> Result<Vec<u8>, String> {
    if !encoded.len().is_multiple_of(2) {
        return Err("visible tree entry has odd-length raw path hex".to_string());
    }
    let mut bytes = Vec::with_capacity(encoded.len() / 2);
    for pair in encoded.as_bytes().chunks(2) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn git_tree_entry_cmp(left: &EncodedTreeEntry, right: &EncodedTreeEntry) -> std::cmp::Ordering {
    let max = std::cmp::max(left.name.len(), right.name.len());
    for index in 0..=max {
        let left_byte = git_tree_sort_byte(left, index);
        let right_byte = git_tree_sort_byte(right, index);
        match left_byte.cmp(&right_byte) {
            std::cmp::Ordering::Equal => continue,
            ordering => return ordering,
        }
    }
    std::cmp::Ordering::Equal
}

fn git_tree_sort_byte(entry: &EncodedTreeEntry, index: usize) -> u8 {
    entry
        .name
        .get(index)
        .copied()
        .unwrap_or(if entry.is_directory { b'/' } else { 0 })
}

fn hex_object_id_bytes(object_id: &str) -> Result<Vec<u8>, String> {
    if !object_id.len().is_multiple_of(2) {
        return Err(format!("object id has odd hex length: {}", object_id));
    }
    let mut bytes = Vec::with_capacity(object_id.len() / 2);
    for pair in object_id.as_bytes().chunks(2) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn is_git_tree_mode(mode: &str) -> bool {
    mode == "40000" || mode == "040000"
}

fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(format!("invalid object id hex byte: {}", byte as char)),
    }
}

fn git_object_id(
    object_hash_algorithm: GitObjectHashAlgorithm,
    kind: &str,
    body: &[u8],
) -> Result<String, String> {
    // Match Git object IDs exactly: hash `"<kind> <len>\0<body>"` with the
    // repository's object format.
    let mut object = Vec::new();
    object.extend_from_slice(kind.as_bytes());
    object.push(b' ');
    object.extend_from_slice(body.len().to_string().as_bytes());
    object.push(0);
    object.extend_from_slice(body);
    Ok(match object_hash_algorithm {
        GitObjectHashAlgorithm::Sha1 => hex_bytes(&Sha1::digest(&object)),
        GitObjectHashAlgorithm::Sha256 => hex_bytes(&Sha256::digest(&object)),
    })
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

pub(super) fn git_object_hash_algorithm(root: &Path) -> Result<GitObjectHashAlgorithm, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-object-format"])
        .output()
        .map_err(|err| format!("failed to run git rev-parse --show-object-format: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to detect git object hash algorithm: {}",
            command_output_trimmed(&output.stderr, "git rev-parse stderr")?
        ));
    }
    let format = command_output_token(&output.stdout, "git rev-parse stdout")?;
    match format {
        "sha1" => Ok(GitObjectHashAlgorithm::Sha1),
        "sha256" => Ok(GitObjectHashAlgorithm::Sha256),
        other => Err(format!("unsupported git object hash algorithm: {}", other)),
    }
}

pub(super) fn scope_entry_path(path: &[u8]) -> String {
    match std::str::from_utf8(path) {
        Ok(path) => path.to_string(),
        Err(_) => {
            let mut output = String::from(RAW_PATH_HEX_PREFIX);
            for byte in path {
                output.push(HEX[(byte >> 4) as usize] as char);
                output.push(HEX[(byte & 0x0f) as usize] as char);
            }
            output
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{visible_tree_oid_from_entries, GitObjectHashAlgorithm};

    #[test] // xpec: A8,UH
    fn visible_tree_hash_rejects_non_leaf_directory_entries() {
        let error = visible_tree_oid_from_entries(
            &["40000 0123456789012345678901234567890123456789\tdir".to_string()],
            GitObjectHashAlgorithm::Sha1,
        )
        .unwrap_err();

        assert_eq!(error, "visible tree entries must be leaf paths");
    }
}
