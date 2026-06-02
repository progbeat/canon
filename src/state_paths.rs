// Canon-owned persistent state is rooted at
// `CANON_STATE_DIR = git rev-parse --git-path canon`.
pub(crate) const CANON_STATE_DIR_GIT_PATH: &str = "canon";

// `${CANON_STATE_DIR}/cache`, resolved through `git rev-parse --git-path`.
pub(crate) const CANON_CACHE_DIR_GIT_PATH: &str = "canon/cache";

// `${CANON_STATE_DIR}/logs`, resolved through `git rev-parse --git-path`.
pub(crate) const CANON_LOG_DIR_GIT_PATH: &str = "canon/logs";
