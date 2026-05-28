use crate::config_types::AgentConfig;
use crate::git::git_head_tree_exists;
use crate::hash::full_scope;
use crate::project::command_output_trimmed;
use crate::scope::{
    effective_ignore_patterns, path_matches_pattern_bytes, sanitize_scope_for_hash,
};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

const HEX: &[u8; 16] = b"0123456789abcdef";
const RAW_PATH_HEX_PREFIX: &str = "\0raw-path-hex:";

type ScopeCacheKey = (PathBuf, Vec<String>, Vec<String>);

#[derive(Default)]
pub(crate) struct VisibleTreeOidCache {
    values: BTreeMap<ScopeCacheKey, Option<String>>,
    // These root-level entries are the only staged-tree Git subprocess
    // results used by `canon check`; scope-specific lookups filter/hash them
    // in memory.
    staged_all_entries: BTreeMap<PathBuf, Vec<String>>,
    staged_root_tree_oids: BTreeMap<PathBuf, String>,
    gate_head_values: BTreeMap<ScopeCacheKey, Option<String>>,
    gate_head_all_entries: BTreeMap<PathBuf, Option<Vec<String>>>,
    head_exists: BTreeMap<PathBuf, bool>,
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

    pub(crate) fn staged_visible_file_count(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<usize, String> {
        let scope = sanitize_scope_for_hash(scope)?;
        let entries = self.staged_scope_entries_from_full_listing(root, agent, &scope)?;
        Ok(entries
            .iter()
            .filter(|entry| !scope_entry_is_tree(entry))
            .count())
    }

    pub(crate) fn missing_staged_scope_paths(
        &mut self,
        root: &Path,
        scope: &[String],
    ) -> Result<Vec<String>, String> {
        let scope = sanitize_scope_for_hash(scope)?;
        if scope == full_scope() {
            return Ok(Vec::new());
        }
        let entries = self.staged_all_scope_entries(root)?;
        Ok(scope
            .into_iter()
            .filter(|base| {
                !entries
                    .iter()
                    .any(|entry| scope_entry_is_within_base(base, entry))
            })
            .collect())
    }

    fn staged_visible_tree_oid_option(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Option<String>, String> {
        let scope = sanitize_scope_for_hash(scope)?;
        let key = scope_cache_key(root, agent, &scope);
        if let Some(hash) = self.values.get(&key) {
            return Ok(hash.clone());
        }
        let root_tree_oid = self.staged_root_tree_oid(root)?;
        let entries = self.staged_all_scope_entries(root)?.clone();
        let visible_entries = filter_visible_scope_entries(&entries, agent, &scope);
        let hash = Some(
            if scope == full_scope() && visible_entries.len() == entries.len() {
                // The visible tree is exactly the staged root tree. Git has already
                // materialized the required object ID, so preserve that native OID
                // instead of serializing and hashing an equivalent synthetic tree.
                root_tree_oid
            } else {
                visible_tree_oid_from_entries(&visible_entries, self.object_hash_algorithm(root)?)?
            },
        );
        self.values.insert(key, hash.clone());
        Ok(hash)
    }

    fn staged_scope_entries_from_full_listing(
        &mut self,
        root: &Path,
        agent: &AgentConfig,
        scope: &[String],
    ) -> Result<Vec<String>, String> {
        // Visible tree OIDs may be requested for many selected, cached, and
        // narrowed scopes during one `canon check`, so cache the full listing
        // once. `visibleTreeOid` hashes the evaluator-visible subset of the
        // scoped tracked tree: canon/evaluator-denied entries are tracked Git
        // entries, but absent from the staged snapshot the evaluator sees.
        let entries = self.staged_all_scope_entries(root)?;
        Ok(filter_visible_scope_entries(entries, agent, scope))
    }

    fn staged_all_scope_entries(&mut self, root: &Path) -> Result<&Vec<String>, String> {
        if !self.staged_all_entries.contains_key(root) {
            let root_tree_oid = self.staged_root_tree_oid(root)?;
            let entries = staged_tree_scope_entries(root, &root_tree_oid)?;
            self.staged_all_entries.insert(root.to_path_buf(), entries);
        }
        self.staged_all_entries
            .get(root)
            .ok_or_else(|| "failed to cache staged scope entries".to_string())
    }

    fn staged_root_tree_oid(&mut self, root: &Path) -> Result<String, String> {
        if let Some(oid) = self.staged_root_tree_oids.get(root) {
            return Ok(oid.clone());
        }
        let oid = git_write_index_tree_oid(root)?;
        self.staged_root_tree_oids
            .insert(root.to_path_buf(), oid.clone());
        Ok(oid)
    }

    #[cfg(all(test, unix))]
    fn staged_scope_entries(
        &mut self,
        root: &Path,
        scope: &[String],
    ) -> Result<Vec<String>, String> {
        staged_scope_entries_for_scope(root, scope)
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
        // Gate compares staged cache records with the committed HEAD tree to
        // tell whether a cached failure is a new regression. The same scoped
        // tree object construction is used for answer-history `visibleTreeOid`
        // records produced by `staged_visible_tree_oid`.
        let object_hash_algorithm = self.object_hash_algorithm(root)?;
        let hash = self
            .gate_head_entries_from_full_listing(root)?
            .map(|entries| filter_visible_scope_entries(&entries, agent, &scope))
            .map(|entries| visible_tree_oid_from_entries(&entries, object_hash_algorithm))
            .transpose()?;
        self.gate_head_values.insert(key, hash.clone());
        Ok(hash)
    }

    fn gate_head_entries_from_full_listing(
        &mut self,
        root: &Path,
    ) -> Result<Option<Vec<String>>, String> {
        if self.gate_head_all_entries.contains_key(root) {
            return Ok(self.gate_head_all_entries.get(root).cloned().flatten());
        }
        if !self.git_has_head(root)? {
            self.gate_head_all_entries.insert(root.to_path_buf(), None);
            return Ok(None);
        }
        let entries = head_scope_entries_for_existing_head(root).map(Some)?;
        self.gate_head_all_entries
            .insert(root.to_path_buf(), entries.clone());
        Ok(entries)
    }

    pub(crate) fn git_has_head(&mut self, root: &Path) -> Result<bool, String> {
        if let Some(has_head) = self.head_exists.get(root) {
            return Ok(*has_head);
        }
        let has_head = git_head_tree_exists(root)?;
        self.head_exists.insert(root.to_path_buf(), has_head);
        Ok(has_head)
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
    let scope = sanitize_scope_for_hash(scope)?;
    let object_hash_algorithm = git_object_hash_algorithm(root)?;
    head_scope_entries(root, &scope).and_then(|entries| {
        entries
            .map(|entries| {
                visible_tree_oid_from_entries(
                    &filter_visible_scope_entries(&entries, agent, &scope),
                    object_hash_algorithm,
                )
            })
            .transpose()
    })
}

fn visible_tree_oid_from_entries(
    entries: &[String],
    object_hash_algorithm: GitObjectHashAlgorithm,
) -> Result<String, String> {
    // `visibleTreeOid` is a Git-compatible tree object ID. We rebuild the scoped
    // tree from Git-reported modes/object IDs, then hash the canonical `tree
    // <len>\0<body>` bytes with the repository's object hash algorithm.
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

#[cfg(test)]
pub(crate) fn head_scope_entries(
    root: &Path,
    scope: &[String],
) -> Result<Option<Vec<String>>, String> {
    if !git_head_tree_exists(root)? {
        return Ok(None);
    }
    head_scope_entries_for_existing_head(root)
        .map(|entries| filter_scope_entries(&entries, scope))
        .map(Some)
}

pub(crate) fn head_scope_entries_for_existing_head(root: &Path) -> Result<Vec<String>, String> {
    git_scope_entries(root, GitScopeListing::Head)
}

#[cfg(all(test, unix))]
fn staged_scope_entries_for_scope(root: &Path, scope: &[String]) -> Result<Vec<String>, String> {
    let root_tree_oid = git_write_index_tree_oid(root)?;
    staged_tree_scope_entries(root, &root_tree_oid)
        .map(|entries| filter_scope_entries(&entries, scope))
}

#[derive(Clone, Copy)]
enum GitScopeListing {
    #[cfg(test)]
    Index,
    Head,
    StagedTree,
}

fn git_scope_entries(root: &Path, listing: GitScopeListing) -> Result<Vec<String>, String> {
    git_tree_scope_entries(root, "HEAD", listing)
}

fn staged_tree_scope_entries(root: &Path, root_tree_oid: &str) -> Result<Vec<String>, String> {
    git_tree_scope_entries(root, root_tree_oid, GitScopeListing::StagedTree)
}

fn git_write_index_tree_oid(root: &Path) -> Result<String, String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).arg("write-tree");
    let output = command
        .output()
        .map_err(|err| format!("failed to run git write-tree: {}", err))?;
    if !output.status.success() {
        return Err(format!(
            "failed to write staged tree: {}",
            command_output_trimmed(&output.stderr, "git write-tree stderr")?
        ));
    }
    let oid = command_output_trimmed(&output.stdout, "git write-tree stdout")?;
    if oid.is_empty() {
        return Err("git write-tree returned an empty tree object id".to_string());
    }
    Ok(oid.to_string())
}

fn git_tree_scope_entries(
    root: &Path,
    treeish: &str,
    listing: GitScopeListing,
) -> Result<Vec<String>, String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .arg("--literal-pathspecs")
        .args(["ls-tree", "-z", "-r", "-t", treeish, "--"]);
    let output = command
        .output()
        .map_err(|err| format!("failed to run {}: {}", listing.command_name(), err))?;
    if !output.status.success() {
        return Err(format!(
            "{}: {}",
            listing.inspect_error(),
            command_output_trimmed(&output.stderr, listing.stderr_label())?
        ));
    }
    normalized_git_scope_entries(&output.stdout, listing)
}

fn normalized_git_scope_entries(
    stdout: &[u8],
    listing: GitScopeListing,
) -> Result<Vec<String>, String> {
    let mut entries = Vec::new();
    for record in stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        if let Some((metadata, path)) = split_raw_scope_record(record, listing.command_name())? {
            entries.push(normalize_git_scope_metadata(metadata, path, listing)?);
        }
    }
    sort_scope_entries(&mut entries);
    Ok(entries)
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

fn filter_visible_scope_entries(
    entries: &[String],
    agent: &AgentConfig,
    scope: &[String],
) -> Vec<String> {
    // `visibleTreeOid` fingerprints the glossary visible tree. The base scope
    // is the latest verified q-scope or full project scope; normalized agent
    // ignore patterns are then applied last as exclusions. Tracked entries
    // outside either part cannot support that evaluator answer, so they are
    // outside the cache-reuse fingerprint.
    let deny_patterns = effective_ignore_patterns(agent);
    let visible_entries = entries
        .iter()
        .map(|entry| scope_entry_is_visible(entry, scope, &deny_patterns))
        .collect::<Vec<_>>();
    let directories_with_non_visible_descendants =
        directories_with_non_visible_descendants(entries, &visible_entries);
    entries
        .iter()
        .zip(visible_entries)
        .filter(|(entry, visible)| {
            // Dropping a Git-reported tree object here does not remove
            // ancestor directory access from the evaluator-visible tree:
            // `TreeNode::insert` rebuilds ancestor directories from visible
            // file entries. We only reuse a tree object's existing OID when it
            // cannot smuggle non-visible descendants into the fingerprint.
            *visible
                && (!scope_entry_is_tree(entry)
                    || tree_oid_entry_has_only_visible_descendants(
                        entry,
                        &directories_with_non_visible_descendants,
                    ))
        })
        .map(|(entry, _)| entry)
        .cloned()
        .collect()
}

fn directories_with_non_visible_descendants(
    entries: &[String],
    visible_entries: &[bool],
) -> BTreeSet<Vec<u8>> {
    let mut directories = BTreeSet::new();
    for (entry, visible) in entries.iter().zip(visible_entries) {
        if *visible {
            continue;
        }
        if let Ok(path) = scope_entry_path_bytes(entry) {
            add_ancestor_directories(&path, &mut directories);
        }
    }
    directories
}

fn add_ancestor_directories(path: &[u8], directories: &mut BTreeSet<Vec<u8>>) {
    let mut end = path.len();
    while let Some(index) = path[..end].iter().rposition(|byte| *byte == b'/') {
        if index > 0 {
            directories.insert(path[..index].to_vec());
        }
        end = index;
    }
}

fn scope_entry_is_visible(entry: &str, scope: &[String], deny_patterns: &[String]) -> bool {
    scope_entry_is_in_scope(entry, scope) && !scope_entry_is_denied(entry, deny_patterns)
}

fn scope_entry_is_in_scope(entry: &str, scope: &[String]) -> bool {
    scope == full_scope()
        || scope
            .iter()
            .any(|base| scope_entry_is_within_base(base, entry))
}

fn scope_entry_is_denied(entry: &str, deny_patterns: &[String]) -> bool {
    let Ok(path) = scope_entry_path_bytes(entry) else {
        return false;
    };
    deny_patterns
        .iter()
        .any(|pattern| path_matches_pattern_bytes(&path, pattern.as_bytes()))
}

fn tree_oid_entry_has_only_visible_descendants(
    entry: &str,
    directories_with_non_visible_descendants: &BTreeSet<Vec<u8>>,
) -> bool {
    let Ok(directory) = scope_entry_path_bytes(entry) else {
        return true;
    };
    !directories_with_non_visible_descendants.contains(&directory)
}

fn scope_entry_is_tree(entry: &str) -> bool {
    let metadata = entry.split_once('\t').map(|(metadata, _)| metadata);
    metadata
        .and_then(|metadata| metadata.split_whitespace().next())
        .is_some_and(is_git_tree_mode)
}

fn scope_entry_path_bytes(entry: &str) -> Result<Vec<u8>, String> {
    let components = visible_tree_path_components(scope_entry_from_normalized_entry(entry))?;
    let mut path = Vec::new();
    for (index, component) in components.iter().enumerate() {
        if index > 0 {
            path.push(b'/');
        }
        path.extend_from_slice(component);
    }
    Ok(path)
}

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

fn scope_entry_from_normalized_entry(entry: &str) -> &str {
    entry
        .split_once('\t')
        .map(|(_, path)| path)
        .unwrap_or(entry)
}

impl GitScopeListing {
    fn command_name(self) -> &'static str {
        match self {
            #[cfg(test)]
            GitScopeListing::Index => "git ls-files",
            GitScopeListing::Head => "git ls-tree",
            GitScopeListing::StagedTree => "git ls-tree",
        }
    }

    fn stderr_label(self) -> &'static str {
        match self {
            #[cfg(test)]
            GitScopeListing::Index => "git ls-files stderr",
            GitScopeListing::Head => "git ls-tree stderr",
            GitScopeListing::StagedTree => "git ls-tree stderr",
        }
    }

    fn inspect_error(self) -> &'static str {
        match self {
            #[cfg(test)]
            GitScopeListing::Index => "failed to inspect staged scope",
            GitScopeListing::Head => "failed to inspect HEAD scope",
            GitScopeListing::StagedTree => "failed to inspect staged scope",
        }
    }

    fn malformed_entry(self) -> &'static str {
        match self {
            #[cfg(test)]
            GitScopeListing::Index => "git index entry",
            GitScopeListing::Head => "git tree entry",
            GitScopeListing::StagedTree => "git tree entry",
        }
    }
}

fn sort_scope_entries(entries: &mut Vec<String>) {
    entries.sort();
    entries.dedup();
}

fn split_raw_scope_record<'a>(
    record: &'a [u8],
    command: &str,
) -> Result<Option<(&'a str, &'a [u8])>, String> {
    let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
        return Ok(None);
    };
    let metadata = std::str::from_utf8(&record[..tab])
        .map_err(|_| format!("{} metadata must be valid UTF-8", command))?;
    Ok(Some((metadata, &record[tab + 1..])))
}

#[cfg(test)]
pub(crate) fn normalize_index_metadata(metadata: &str, path: &[u8]) -> Result<String, String> {
    normalize_git_scope_metadata(metadata, path, GitScopeListing::Index)
}

fn normalize_git_scope_metadata(
    metadata: &str,
    path: &[u8],
    listing: GitScopeListing,
) -> Result<String, String> {
    let path = scope_entry_path(path);
    let mut fields = metadata.split_whitespace();
    let mode = next_scope_metadata_field(&mut fields, listing, &path)?;
    match listing {
        #[cfg(test)]
        GitScopeListing::Index => {
            let object = next_scope_metadata_field(&mut fields, listing, &path)?;
            let stage = next_scope_metadata_field(&mut fields, listing, &path)?;
            if stage == "0" {
                Ok(format!("{} {}\t{}", mode, object, path))
            } else {
                Ok(format!("{} {} {}\t{}", mode, object, stage, path))
            }
        }
        GitScopeListing::Head | GitScopeListing::StagedTree => {
            let kind = next_scope_metadata_field(&mut fields, listing, &path)?;
            let object = next_scope_metadata_field(&mut fields, listing, &path)?;
            Ok(format!(
                "{} {}\t{}",
                normalized_git_tree_mode(mode, kind),
                object,
                path
            ))
        }
    }
}

fn normalized_git_tree_mode<'a>(mode: &'a str, kind: &str) -> &'a str {
    if is_git_tree_mode(mode) || kind == "tree" {
        "40000"
    } else {
        mode
    }
}

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
