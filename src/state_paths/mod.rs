// Canon-owned persistent state is rooted at
// `CANON_STATE_DIR = git rev-parse --git-path canon`.
pub(crate) const CANON_STATE_DIR_GIT_PATH: &str = "canon";

// `${CANON_STATE_DIR}/xpecs`, resolved through `git rev-parse --git-path`.
pub(crate) const CANON_XPECS_DIR_GIT_PATH: &str = "canon/xpecs";

// `${CANON_STATE_DIR}/logs`, resolved through `git rev-parse --git-path`.
pub(crate) const CANON_LOG_DIR_GIT_PATH: &str = "canon/logs";

// `${CANON_STATE_DIR}/live-reports`, resolved through `git rev-parse --git-path`.
pub(crate) const CANON_LIVE_REPORT_DIR_GIT_PATH: &str = "canon/live-reports";
