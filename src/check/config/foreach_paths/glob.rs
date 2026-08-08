use super::path::{path_relative_to_config, resolve_foreach_glob};
use crate::scope::{path_bytes_in_scope, utf8_path_matches_glob};
use std::path::Path;

pub(crate) fn expand_foreach_paths_from_listing(
    config_path: &Path,
    glob: &str,
    staged_paths: &[Vec<u8>],
) -> Result<Vec<String>, String> {
    // [jM] Resolve the document directory as a literal prefix and match only
    // the configured pattern suffix as a glob.
    let resolved_glob = resolve_foreach_glob(config_path, glob)?;
    let foreach_pathspec = format!(":(glob){}", resolved_glob.pattern());
    let foreach_scope = std::slice::from_ref(&foreach_pathspec);
    let mut files = Vec::new();
    for staged_path in staged_paths {
        // [jM,nK] Valid UTF-8 paths use character glob semantics. Invalid
        // paths retain byte matching only to decide whether they would have
        // been selected before reporting that they cannot become a binding.
        match std::str::from_utf8(staged_path) {
            Ok(path) => {
                let path = Path::new(path);
                if let Some(candidate) = resolved_glob.utf8_candidate_suffix(path)? {
                    if utf8_path_matches_glob(&candidate, resolved_glob.pattern()) {
                        files.push(path_relative_to_config(config_path, path)?);
                    }
                }
            }
            Err(_) => {
                if let Some(candidate) = resolved_glob.byte_candidate_suffix(staged_path) {
                    if path_bytes_in_scope(&candidate, foreach_scope)? {
                        return Err(
                            "!foreach matched a non-UTF-8 file path that cannot be bound to `path`"
                                .to_string(),
                        );
                    }
                }
            }
        }
    }
    files.sort();
    Ok(files)
}
