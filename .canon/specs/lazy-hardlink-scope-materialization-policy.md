# Lazy Hardlink Scope Materialization Policy

The **lazy hardlink scope materialization policy** defines how evaluator
working trees are produced from a Git tree.

For one `canon check` invocation, initialize one temporary materialization root:

```
lazy_tree_dir = materialization_root / "lazy"
scopes_dir = materialization_root / "scopes"
unpacked_paths = set()
```

The temporary materialization root should be created on a memory-backed temporary filesystem when the host platform provides one. Otherwise, ordinary temporary storage may be used.

The policy materializes one evaluator working tree for each visible scope:

```python
def materialize_scope(git_tree, scope):
    scoped_tree = git_tree.apply_scope(scope)
    scope_root = os.path.join(scopes_dir, scoped_tree.oid)
    if os.path.exists(scope_root):
        return scope_root
    scoped_paths = set(scoped_tree.entry_paths)
    missing_paths = scoped_paths - unpacked_paths
    if missing_paths:
        archive = git_tree.archive(missing_paths)
        archive.extractall(lazy_tree_dir)
        unpacked_paths.update(missing_paths)
    os.makedirs(scope_root)
    for path in scoped_tree.entry_paths:
        dst_path = scope_root / path
        ensure_parent_dirs(dst_path)
        hardlink_file_or_copy_symlink(lazy_tree_dir / path, dst_path)
    return scope_root
```

*This pseudocode is normative for behavior, not for implementation structure.*
