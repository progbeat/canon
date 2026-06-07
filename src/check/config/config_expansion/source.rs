use crate::git::TreeSource;

#[derive(Clone)]
pub(crate) enum CheckConfigSource {
    Tree(TreeSource),
}

impl CheckConfigSource {
    pub(super) fn tree_source(&self) -> &TreeSource {
        match self {
            CheckConfigSource::Tree(source) => source,
        }
    }
}
