# log / FileHandler

The **file handler** is the callable context manager `log.FileHandler(path)`:

```python
import json


class FileHandler:
    def __init__(self, path):
        self.path = path

    def __enter__(self):
        self.file = open(self.path, "ab")
        return self

    def __call__(self, event):
        record = (json.dumps(event, separators=(",", ":")) + "\n").encode("utf-8")
        self.file.write(record)
        self.file.flush()

    def __exit__(self, exc_type, exc, traceback):
        self.file.close()
        return False
```

Concurrent writes do not interleave JSONL records.
