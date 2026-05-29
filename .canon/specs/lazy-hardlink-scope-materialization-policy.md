# Lazy Hardlink Scope Materialization Policy

The **lazy hardlink scope materialization policy** is a policy for evaluator
thread working trees produced from a Git tree.

For one `canon check` invocation, initialize one temporary materialization root:

```
lazy_tree = materialization_root / "lazy"
scope_roots = materialization_root / "scopes"
unpacked_paths = set()
```

The temporary materialization root should be created on a memory-backed temporary filesystem when the host platform provides one. Otherwise, ordinary temporary storage may be used.

The policy materializes one evaluator working tree for each visible scope:

```
def materialize_scope(git_tree, scope):
    scope_paths = {
        path
        for path in file_entries(git_tree)
        if path is in scope
    }

    # The loops below define semantics; implementation work across paths should
    # use batch operations rather than one external command per file.
    for path in scope_paths - unpacked_paths:
        blob = read_blob(git_tree, path)
        mode = git_tree.mode(path)
        write_regular_file(lazy_tree / path, blob.contents, mode)
        unpacked_paths.add(path)

    if scope is the full project scope:
        return lazy_tree

    scope_root = create_directory_under(scope_roots)
    for path in scope_paths:
        hardlink(lazy_tree / path, scope_root / path)
    return scope_root
```

If `path` is a symlink entry in `git_tree`, `blob.contents` is the link target
text and `write_regular_file` still writes a regular file.

The returned evaluator working tree contains exactly the files in `scope_paths`
and their ancestor directories. Files outside `scope_paths` are absent.

Returned evaluator working trees do not contain generated `.git` metadata.
