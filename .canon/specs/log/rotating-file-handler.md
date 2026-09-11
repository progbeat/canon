# log / RotatingFileHandler

The rotating file handler is the callable `log.RotatingFileHandler(directory, max_bytes, file_count=8)`; `max_bytes` and `file_count` are positive integers.

Creating the handler creates the directory if needed and opens the active file for append.
The open stream is reused between records; when the active file changes, including through another writer's rotation, the old stream is closed and the current active file is opened for append.

Calling the handler with an event appends it as one complete flushed JSONL record, preserving its fields and JSON value types.

Managed files are `0.jsonl` through `<file_count - 1>.jsonl`, with `0.jsonl` active and higher numbers older.
Before an append would take a nonempty active file above `max(1, floor(max_bytes / file_count))` bytes, files rotate toward higher numbers, removing the oldest when necessary.
Successful appends keep the combined directory size within `max_bytes`, removing managed files oldest first when necessary.
A record larger than `max_bytes` fails before any existing record is removed.
Concurrent writers preserve complete JSONL records, rotation order, and the directory size bound.
