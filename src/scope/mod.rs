mod compare;
mod ignore;
mod normalize;
mod pathspec;
mod visible;

// Scope owns repo-path normalization, q-scope/visible-scope conversion,
// configured ignore expansion, Git pathspec parsing, and byte-level scope
// matching. Visible-tree hashing/materialization, q-scope verification, and
// evaluator-thread reuse live in their owning components.

pub(crate) use compare::scope_is_within;
pub(crate) use ignore::effective_ignore_patterns;
pub(crate) use normalize::{normalize_repo_path, sanitize_scope};
pub(crate) use pathspec::{path_bytes_in_scope, pathspec_is_exclude};
pub(crate) use visible::visible_scope;
