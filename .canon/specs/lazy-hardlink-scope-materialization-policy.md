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
    scoped_tree = git_tree.limit(pathspecs=scope)
    scope_root = os.path.join(scopes_dir, scoped_tree.oid)
    if os.path.exists(scope_root):
        return scope_root
    scoped_paths = set(scoped_tree.entry_paths)
    missing_paths = scoped_paths - unpacked_paths
    for path in missing_paths:
        dst_path = lazy_tree_dir / path
        git_tree.extract(path, dst=dst_path)
        remove_write_permissions(dst_path, follow_symlinks=False)
        unpacked_paths.add(path)

    def dfs(prefix):
        os.makedirs(scope_root / prefix)
        for name in scoped_tree.children(prefix):
            path = prefix / name
            if scoped_tree.is_dir(path):
                dfs(path)
            else:
                hardlink_file_or_copy_symlink(lazy_tree_dir / path, scope_root / path)
        os.chmod(scope_root / prefix, 0o555)  # to prevent accidental modifications to materialized trees

    dfs(".")
    return scope_root
```

*This pseudocode is normative for behavior, not for implementation structure.*
