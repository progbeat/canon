use super::program::{head_tracked_files, staged_tracked_files, StagedTrackedFile};
use super::tree_source::TreeSource;
use crate::config_types::AgentConfig;
#[cfg(test)]
use crate::hash::full_scope;
use crate::project::command_output_trimmed;
use crate::scope::{
    effective_ignore_patterns, is_denied_path_bytes, path_bytes_in_scope, sanitize_scope_for_hash,
};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

// Cache-spec ownership note: this module implements only the `visibleTreeOid`
// fingerprint. Answer-history storage, JSONL rendering, append, and compaction
// live under `history`, so whole Cache-spec review must inspect those modules
// in addition to this one.
const HEX: &[u8; 16] = b"0123456789abcdef";
const RAW_PATH_HEX_PREFIX: &str = "\0raw-path-hex:";

type ScopeCacheKey = (PathBuf, Vec<String>, Vec<String>);
type SourceScopeCacheKey = (PathBuf, String, Vec<String>, Vec<String>);
type SourceFilesCacheKey = (PathBuf, String);

#[derive(Default)]
pub(crate) struct VisibleTreeOidCache {
    staged_tree_oids: BTreeMap<ScopeCacheKey, Option<String>>,
    tree_source_oids: BTreeMap<SourceScopeCacheKey, String>,
    tree_source_entries: BTreeMap<SourceScopeCacheKey, Vec<String>>,
    tree_source_files: BTreeMap<SourceFilesCacheKey, Result<Vec<StagedTrackedFile>, String>>,
    staged_entries: BTreeMap<ScopeCacheKey, Vec<String>>,
    staged_files: BTreeMap<PathBuf, Result<Vec<StagedTrackedFile>, String>>,
    gate_head_values: BTreeMap<ScopeCacheKey, Option<String>>,
    head_files: BTreeMap<PathBuf, Result<Option<Vec<StagedTrackedFile>>, String>>,
    object_hash_algorithms: BTreeMap<PathBuf, GitObjectHashAlgorithm>,
}

impl VisibleTreeOidCache {
    pub(crate) fn new() -> VisibleTreeOidCache {
        VisibleTreeOidCache::default()
    }

    pub(crate) fn staged_visible_tree_oid(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<String, String> {
        self.staged_visible_tree_oid_option(root, agent, scope)?
            .ok_or("failed to hash staged scope".to_string())
    }

    pub(crate) fn visible_tree_oid(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<String, String> {
        match source {
            TreeSource::Staged => self.staged_visible_tree_oid(root, agent, scope),
            TreeSource::Git { .. } => self.git_tree_visible_tree_oid(root, source, agent, scope),
        }
    }

    pub(crate) fn staged_visible_file_count(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<usize, String> {
        let scope = sanitize_scope_for_hash(scope)?;
        let entries = self.staged_visible_scope_entries(root, agent, &scope)?;
        Ok(entries
            .iter()
            .filter(|entry| !scope_entry_is_tree(entry))
            .count())
    }

    pub(crate) fn visible_file_count(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<usize, String> {
        match source {
            TreeSource::Staged => self.staged_visible_file_count(root, agent, scope),
            TreeSource::Git { .. } => {
                let scope = sanitize_scope_for_hash(scope)?;
                let entries = self.git_tree_visible_scope_entries(root, source, agent, &scope)?;
                Ok(entries
                    .iter()
                    .filter(|entry| !scope_entry_is_tree(entry))
                    .count())
            }
        }
    }

    pub(crate) fn repository_native_object_oid_hex_len(
        &mut self,
        root: &Path,
    ) -> Result<usize, String> {
        Ok(git_object_oid_hex_len(self.object_hash_algorithm(root)?))
    }

    fn staged_visible_tree_oid_option(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Option<String>, String> {
        let scope = sanitize_scope_for_hash(scope)?;
        let key = scope_cache_key(root, agent, &scope);
        if let Some(hash) = self.staged_tree_oids.get(&key) {
            return Ok(hash.clone());
        }
        let visible_entries = self.staged_visible_scope_entries(root, agent, &scope)?;
        let hash = Some(visible_tree_oid_from_entries(
            &visible_entries,
            self.object_hash_algorithm(root)?,
        )?);
        self.staged_tree_oids.insert(key, hash.clone());
        Ok(hash)
    }

    fn git_tree_visible_tree_oid(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<String, String> {
        let scope = sanitize_scope_for_hash(scope)?;
        let key = source_scope_cache_key(root, source, agent, &scope);
        if let Some(hash) = self.tree_source_oids.get(&key) {
            return Ok(hash.clone());
        }
        let visible_entries = self.git_tree_visible_scope_entries(root, source, agent, &scope)?;
        let hash =
            visible_tree_oid_from_entries(&visible_entries, self.object_hash_algorithm(root)?)?;
        self.tree_source_oids.insert(key, hash.clone());
        Ok(hash)
    }

    fn git_tree_visible_scope_entries(
        &mut self,
        root: &Path,
        source: &TreeSource,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Vec<String>, String> {
        let key = source_scope_cache_key(root, source, agent, scope);
        if let Some(entries) = self.tree_source_entries.get(&key) {
            return Ok(entries.clone());
        }
        // Keep direct git subprocesses independent of the number of scopes or
        // history records: list each tree source once, then filter scopes here.
        let files = self.tree_source_files(root, source)?;
        let entries = visible_scope_entries_from_files(&files, agent, scope);
        self.tree_source_entries.insert(key, entries.clone());
        Ok(entries)
    }

    fn staged_visible_scope_entries(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Vec<String>, String> {
        let key = scope_cache_key(root, agent, scope);
        if let Some(entries) = self.staged_entries.get(&key) {
            return Ok(entries.clone());
        }
        // Same bound for the staged index: one `git ls-files` result is cached
        // for the run, and every scope is derived in memory.
        let files = self.staged_files(root)?;
        let entries = visible_scope_entries_from_files(&files, agent, scope);
        self.staged_entries.insert(key, entries.clone());
        Ok(entries)
    }

    fn staged_files(&mut self, root: &Path) -> Result<Vec<StagedTrackedFile>, String> {
        if let Some(files) = self.staged_files.get(root) {
            return files.clone();
        }
        let files = staged_tracked_files(root);
        self.staged_files.insert(root.to_path_buf(), files.clone());
        files
    }

    fn tree_source_files(
        &mut self,
        root: &Path,
        source: &TreeSource,
    ) -> Result<Vec<StagedTrackedFile>, String> {
        let key = (root.to_path_buf(), source.cache_key());
        if let Some(files) = self.tree_source_files.get(&key) {
            return files.clone();
        }
        let files = source.tracked_files(root);
        self.tree_source_files.insert(key, files.clone());
        files
    }

    #[cfg(test)]
    pub(crate) fn staged_entries_cache_len(&self) -> usize {
        self.staged_entries.len()
    }

    #[cfg(all(test, unix))]
    fn staged_scope_entries(
        &mut self,
        root: &Path,
        scope: &[String],
    ) -> Result<Vec<String>, String> {
        let scope = sanitize_scope_for_hash(scope)?;
        let files = self.staged_files(root)?;
        Ok(scope_entries_from_files(&files, &scope))
    }

    pub(crate) fn gate_head_tree_fingerprint(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Option<String>, String> {
        let scope = sanitize_scope_for_hash(scope)?;
        let key = scope_cache_key(root, agent, &scope);
        if let Some(hash) = self.gate_head_values.get(&key) {
            return Ok(hash.clone());
        }
        let object_hash_algorithm = self.object_hash_algorithm(root)?;
        let hash = self
            .head_visible_scope_entries(root, agent, &scope)?
            .map(|entries| visible_tree_oid_from_entries(&entries, object_hash_algorithm))
            .transpose()?;
        self.gate_head_values.insert(key, hash.clone());
        Ok(hash)
    }

    fn head_visible_scope_entries(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Option<Vec<String>>, String> {
        Ok(self
            .head_files(root)?
            .map(|files| visible_scope_entries_from_files(&files, agent, scope)))
    }

    fn head_files(&mut self, root: &Path) -> Result<Option<Vec<StagedTrackedFile>>, String> {
        if let Some(files) = self.head_files.get(root) {
            return files.clone();
        }
        let files = head_tracked_files(root);
        self.head_files.insert(root.to_path_buf(), files.clone());
        files
    }

    fn object_hash_algorithm(&mut self, root: &Path) -> Result<GitObjectHashAlgorithm, String> {
        if let Some(algorithm) = self.object_hash_algorithms.get(root) {
            return Ok(*algorithm);
        }
        let algorithm = git_object_hash_algorithm(root)?;
        self.object_hash_algorithms
            .insert(root.to_path_buf(), algorithm);
        Ok(algorithm)
    }
}

fn scope_cache_key(root: &Path, agent: &AgentConfig, scope: &[String]) -> ScopeCacheKey {
    let mut deny_patterns = effective_ignore_patterns(agent);
    deny_patterns.sort();
    deny_patterns.dedup();
    (root.to_path_buf(), scope.to_vec(), deny_patterns)
}

fn source_scope_cache_key(
    root: &Path,
    source: &TreeSource,
    agent: &AgentConfig,
    scope: &[String],
) -> SourceScopeCacheKey {
    let mut deny_patterns = effective_ignore_patterns(agent);
    deny_patterns.sort();
    deny_patterns.dedup();
    (
        root.to_path_buf(),
        source.cache_key(),
        scope.to_vec(),
        deny_patterns,
    )
}

#[cfg(test)]
pub(crate) fn staged_visible_tree_oid(
    root: &Path,
    agent: &AgentConfig,
    scope: &[String],
) -> Result<String, String> {
    VisibleTreeOidCache::new().staged_visible_tree_oid(root, agent, scope)
}

#[cfg(test)]
pub(crate) fn gate_head_tree_fingerprint(
    root: &Path,
    agent: &AgentConfig,
    scope: &[String],
) -> Result<Option<String>, String> {
    VisibleTreeOidCache::new().gate_head_tree_fingerprint(root, agent, scope)
}

fn visible_tree_oid_from_entries(
    entries: &[String],
    object_hash_algorithm: GitObjectHashAlgorithm,
) -> Result<String, String> {
    // `visibleTreeOid` is a Git-compatible tree object ID. We rebuild the scoped
    // tree from Git-reported modes/object IDs, then hash the canonical `tree
    // <len>\0<body>` bytes with the repository's object hash algorithm.
    //
    // The entries come from `git ls-tree -r -t`, so fully covered directories
    // carry Git's existing tree object ID. `TreeNode::insert` preserves those as
    // `DirectoryOid` and ignores redundant descendants, reusing Git's subtree
    // OIDs whenever the visible tree contains a complete directory. Only the
    // synthetic root or ancestors that Git does not already report are serialized
    // and hashed here.
    let mut tree = TreeNode::default();
    for entry in entries {
        let parsed = parse_visible_tree_entry(entry)?;
        tree.insert(&parsed.path, parsed.mode, parsed.object_id)?;
    }
    tree.oid(object_hash_algorithm)
}

#[cfg(test)]
pub(crate) fn sha1_visible_tree_oid_from_entries(entries: &[String]) -> Result<String, String> {
    visible_tree_oid_from_entries(entries, GitObjectHashAlgorithm::Sha1)
}

#[derive(Clone, Copy)]
enum GitObjectHashAlgorithm {
    Sha1,
    Sha256,
}

pub(crate) fn git_object_oid_has_known_shape(object_id: &str) -> bool {
    [GitObjectHashAlgorithm::Sha1, GitObjectHashAlgorithm::Sha256]
        .into_iter()
        .any(|algorithm| git_object_oid_matches_algorithm(object_id, algorithm))
}

pub(crate) fn repository_native_object_oid_hex_len(root: &Path) -> Result<usize, String> {
    Ok(git_object_oid_hex_len(git_object_hash_algorithm(root)?))
}

pub(crate) fn git_object_oid_has_hex_len(object_id: &str, hex_len: usize) -> bool {
    object_id.len() == hex_len
        && object_id
            .as_bytes()
            .iter()
            .all(|byte| hex_nibble(*byte).is_ok())
}

fn git_object_oid_matches_algorithm(object_id: &str, algorithm: GitObjectHashAlgorithm) -> bool {
    git_object_oid_has_hex_len(object_id, git_object_oid_hex_len(algorithm))
}

fn git_object_oid_hex_len(algorithm: GitObjectHashAlgorithm) -> usize {
    match algorithm {
        GitObjectHashAlgorithm::Sha1 => 40,
        GitObjectHashAlgorithm::Sha256 => 64,
    }
}

#[derive(Default)]
struct TreeNode {
    entries: BTreeMap<Vec<u8>, TreeEntry>,
}

enum TreeEntry {
    File { mode: String, object_id: String },
    Directory(TreeNode),
    // Fully covered directories reuse the tree object ID that Git reports.
    // Child entries under this directory are redundant for the scoped tree.
    DirectoryOid { object_id: String },
}

struct VisibleTreeEntry {
    mode: String,
    object_id: String,
    path: Vec<Vec<u8>>,
}

impl TreeNode {
    fn insert(&mut self, path: &[Vec<u8>], mode: String, object_id: String) -> Result<(), String> {
        let Some((name, rest)) = path.split_first() else {
            return Err("visible tree entry path must not be empty".to_string());
        };
        if rest.is_empty() {
            let entry = if is_git_tree_mode(&mode) {
                TreeEntry::DirectoryOid { object_id }
            } else {
                TreeEntry::File { mode, object_id }
            };
            self.entries.insert(name.clone(), entry);
            return Ok(());
        }
        let entry = self
            .entries
            .entry(name.clone())
            .or_insert_with(|| TreeEntry::Directory(TreeNode::default()));
        match entry {
            TreeEntry::Directory(directory) => directory.insert(rest, mode, object_id),
            TreeEntry::DirectoryOid { .. } => Ok(()),
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
            TreeEntry::DirectoryOid { object_id } => Ok(EncodedTreeEntry {
                name: name.to_vec(),
                mode: "40000".to_string(),
                object_id: hex_object_id_bytes(object_id)?,
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

fn parse_visible_tree_entry(entry: &str) -> Result<VisibleTreeEntry, String> {
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
    // repository's object format. This is not Canon's base64url `hash_120`.
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
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        output.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(all(test, unix))]
pub(crate) fn staged_scope_entries(root: &Path, scope: &[String]) -> Result<Vec<String>, String> {
    VisibleTreeOidCache::new().staged_scope_entries(root, scope)
}

fn visible_scope_entries_from_files(
    files: &[StagedTrackedFile],
    agent: &AgentConfig,
    scope: &[String],
) -> Vec<String> {
    let visible_files = files
        .iter()
        .filter(|file| file.is_blob_file_entry())
        .filter(|file| path_bytes_in_scope(&file.path, scope))
        .filter(|file| !is_denied_path_bytes(agent, &file.path))
        .collect::<Vec<_>>();
    tracked_files_scope_entries(&visible_files)
}

#[cfg(all(test, unix))]
fn scope_entries_from_files(files: &[StagedTrackedFile], scope: &[String]) -> Vec<String> {
    let visible_files = files
        .iter()
        .filter(|file| path_bytes_in_scope(&file.path, scope))
        .collect::<Vec<_>>();
    tracked_files_scope_entries(&visible_files)
}

fn tracked_files_scope_entries(files: &[&StagedTrackedFile]) -> Vec<String> {
    let mut entries = files
        .iter()
        .map(|file| {
            format!(
                "{} {}\t{}",
                file.mode,
                file.object_id,
                scope_entry_path(&file.path)
            )
        })
        .collect::<Vec<_>>();
    sort_scope_entries(&mut entries);
    entries
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum GitScopeListing {
    Index,
}

fn git_object_hash_algorithm(root: &Path) -> Result<GitObjectHashAlgorithm, String> {
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
    let format = command_output_trimmed(&output.stdout, "git rev-parse stdout")?;
    match format {
        "sha1" => Ok(GitObjectHashAlgorithm::Sha1),
        "sha256" => Ok(GitObjectHashAlgorithm::Sha256),
        other => Err(format!("unsupported git object hash algorithm: {}", other)),
    }
}

#[cfg(test)]
fn filter_scope_entries(entries: &[String], scope: &[String]) -> Vec<String> {
    if scope == full_scope() {
        return entries.to_vec();
    }
    entries
        .iter()
        .filter(|entry| {
            scope
                .iter()
                .any(|base| scope_entry_is_within_base(base, entry))
        })
        .cloned()
        .collect()
}

fn scope_entry_is_tree(entry: &str) -> bool {
    let metadata = entry.split_once('\t').map(|(metadata, _)| metadata);
    metadata
        .and_then(|metadata| metadata.split_whitespace().next())
        .is_some_and(is_git_tree_mode)
}

#[cfg(test)]
fn scope_entry_is_within_base(base: &str, entry: &str) -> bool {
    let path = scope_entry_from_normalized_entry(entry);
    let Ok(base_components) = visible_tree_path_components(base) else {
        return false;
    };
    let Ok(path_components) = visible_tree_path_components(path) else {
        return false;
    };
    path_components.starts_with(&base_components)
}

#[cfg(test)]
fn scope_entry_from_normalized_entry(entry: &str) -> &str {
    entry
        .split_once('\t')
        .map(|(_, path)| path)
        .unwrap_or(entry)
}

#[cfg(test)]
impl GitScopeListing {
    fn malformed_entry(self) -> &'static str {
        match self {
            GitScopeListing::Index => "git index entry",
        }
    }
}

fn sort_scope_entries(entries: &mut Vec<String>) {
    entries.sort();
    entries.dedup();
}

#[cfg(test)]
pub(crate) fn normalize_index_metadata(metadata: &str, path: &[u8]) -> Result<String, String> {
    normalize_git_scope_metadata(metadata, path, GitScopeListing::Index)
}

#[cfg(test)]
fn normalize_git_scope_metadata(
    metadata: &str,
    path: &[u8],
    listing: GitScopeListing,
) -> Result<String, String> {
    let path = scope_entry_path(path);
    let mut fields = metadata.split_whitespace();
    let mode = next_scope_metadata_field(&mut fields, listing, &path)?;
    match listing {
        GitScopeListing::Index => {
            let object = next_scope_metadata_field(&mut fields, listing, &path)?;
            let stage = next_scope_metadata_field(&mut fields, listing, &path)?;
            if stage == "0" {
                Ok(format!("{} {}\t{}", mode, object, path))
            } else {
                Ok(format!("{} {} {}\t{}", mode, object, stage, path))
            }
        }
    }
}

#[cfg(test)]
fn next_scope_metadata_field<'a>(
    fields: &mut std::str::SplitWhitespace<'a>,
    listing: GitScopeListing,
    path: &str,
) -> Result<&'a str, String> {
    fields
        .next()
        .ok_or_else(|| format!("malformed {} for {}", listing.malformed_entry(), path))
}

fn scope_entry_path(path: &[u8]) -> String {
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
    use super::*;

    #[test]
    fn parent_scope_matches_raw_hex_child_entry() {
        let entry = normalize_git_scope_metadata(
            "100644 0123456789012345678901234567890123456789 0",
            b"dir/nonutf8-\xff.txt",
            GitScopeListing::Index,
        )
        .unwrap();

        assert!(entry.contains(RAW_PATH_HEX_PREFIX));
        assert_eq!(
            filter_scope_entries(std::slice::from_ref(&entry), &["dir".to_string()]),
            vec![entry]
        );
    }
}
