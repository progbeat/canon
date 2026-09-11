# Settings

`settings` owns resolution of shared application parameters and exposes their immutable values process-wide.

```
namespace settings:
    CANON_STATE_DIR = env['CANON_STATE_DIR'] ?? git('rev-parse', '--git-path', 'canon')
    LOGS_DIR = CANON_STATE_DIR / 'logs'
    XPECS_DIR = CANON_STATE_DIR / 'xpecs'
    LOGS_MAX_SIZE = parse_bytes(git_config('canon.logs.maxSize') ?? '0M')  # LOGS_DIR size limit; supports M/G suffixes
```

*This pseudocode is normative for behavior, not for implementation structure.* Calls describe value sources and transformations, not required helpers or subprocess boundaries.

Git-backed parameters, including all `canon.*` settings, are resolved together on first use with at most one Git subprocess invocation; subsequent accesses reuse that snapshot for the rest of the process.

All canon-owned non-temporary persistent state is stored only under `CANON_STATE_DIR`.
