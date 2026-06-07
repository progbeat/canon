use std::fs;
use std::path::Path;

pub(crate) fn mirror_evaluator_codex_home_file(source: &Path, target: &Path) -> Result<(), String> {
    fs::copy(source, target).map(|_| ()).map_err(|err| {
        format!(
            "failed to copy evaluator CODEX_HOME file {} from {}: {}",
            target.display(),
            source.display(),
            err
        )
    })
}

pub(crate) fn move_path(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target).map_err(|err| {
        format!(
            "failed to move isolated path {} to {}: {}",
            source.display(),
            target.display(),
            err
        )
    })
}
