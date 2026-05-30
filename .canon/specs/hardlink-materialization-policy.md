# Hardlink Materialization Policy

The **hardlink materialization policy** defines how evaluator working trees are produced from a Git tree.

```python
class HardlinkMaterializationPolicy:
    def __init__(self):
        tmp_dir = os.environ.get("CANON_TREE_CACHE_DIR") or make_temp_dir()
        self.lazy_tree_dir = tmp_dir / "lazy"
        self.trees_dir = tmp_dir / "trees"
        self.unpacked_paths = set()

    def materialize(self, visible_tree):
        target_root = self.trees_dir / visible_tree.oid
        if os.path.exists(target_root):
            return target_root
        visible_paths = set(visible_tree.entry_paths)
        missing_paths = visible_paths - self.unpacked_paths
        for path in missing_paths:
            dst_path = self.lazy_tree_dir / path
            visible_tree.extract(path, dst=dst_path)
            remove_write_permissions(dst_path, follow_symlinks=False)
            self.unpacked_paths.add(path)

        def dfs(prefix):
            os.makedirs(target_root / prefix)
            for name in visible_tree.children(prefix):
                path = prefix / name
                if visible_tree.is_dir(path):
                    dfs(path)
                else:
                    hardlink_file_or_copy_symlink(self.lazy_tree_dir / path, target_root / path)
            os.chmod(target_root / prefix, 0o555)

        dfs(".")
        return target_root
```

*This pseudocode is normative for behavior, not for implementation structure.*
