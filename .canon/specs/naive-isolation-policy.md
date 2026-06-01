# Naive Isolation Policy

The **naive isolation policy** is an isolation policy that relies on filesystem move and chmod operations.

```python
class NaiveIsolationPolicy:
    def __init__(self):
        self.secret_dir = os.environ.get("CANON_SECRET_DIR")
        self.sandbox_dir = os.environ.get("CANON_SANDBOX_DIR") or make_temp_dir()
        self.counter = 0
        if self.secret_dir:
            self.secret_dir_mode = stat_mode(self.secret_dir)

    @contextmanager
    def isolate(self, path):
        """Isolate the given path by moving it to a sandbox directory and optionally removing permissions from a secret directory."""
        assert self.secret_dir is None or is_subpath(path, self.secret_dir), "cannot isolate path outside of secret dir"
        dst_path = os.path.join(self.sandbox_dir, format(self.counter, "X"))
        self.counter += 1
        assert not exists(dst_path), "counter collision in sandbox isolation"
        with ExitStack() as stack:
            move(path, dst_path)
            stack.callback(move, dst_path, path)
            if self.secret_dir:
                chmod(self.secret_dir, 0o000)
                stack.callback(chmod, self.secret_dir, self.secret_dir_mode)
            yield dst_path
```
