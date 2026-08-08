use super::template_error;
use crate::process_cwd::with_current_dir;
use minijinja::Error;
use std::path::Path;

pub(crate) fn render_with_repository_cwd<F>(root: &Path, render: F) -> Result<String, Error>
where
    F: FnOnce(&Path) -> Result<String, Error>,
{
    with_current_dir(root, render).map_err(template_error)?
}
