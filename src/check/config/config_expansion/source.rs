use crate::git::TreeSource;

#[derive(Clone)]
pub(crate) enum CheckConfigSource {
    Tree(TreeSource),
    InPlace,
}

impl CheckConfigSource {
    pub(crate) fn cache_key(&self) -> String {
        match self {
            CheckConfigSource::Tree(source) => source.cache_key(),
            CheckConfigSource::InPlace => ":in-place".to_string(),
        }
    }
}
