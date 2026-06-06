// Hardlink materialization is split by responsibility:
// `paths` owns the policy's tmp_dir selection, while `worktree` owns
// lazy_tree_dir, trees_dir, unpacked_paths, and materialize().
mod paths;
mod worktree;

pub(crate) use worktree::StagedWorktreeView;
