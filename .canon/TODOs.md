# TODOs

- [ ] Switch to `ignore` crate instead of git pathspecs for scope matching.
- [ ] Detect if `canon` was invoked by an agent and forbid modifying files like README.md, AGENTS.md, etc.


```yaml
- q: |
    A well-structured implementation is organized as self-contained, platform-independent components with minimal necessary public interfaces. A component owns the source locations that define, assemble, and expose its behavior, including its internal source structure. Other components interact with it only through its public interface. A component may span multiple source locations, but those locations must be clearly part of the component's own declared implementation boundary.
    Platform-specific behavior belongs in implementation details behind the component boundary. A component's public contract should not require callers to know, preserve, branch on, or depend on platform-specific implementation structure.
    A component should be portable enough that moving it to another project requires moving only that component's own implementation boundary, not unrelated source locations that define, modify, assemble, expose, or control its behavior.
    Does the codebase contain any source location that violates these design principles?
  a: "no"
  preset: smart
```
