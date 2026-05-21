# Examples

Initialize canon in a project:

```sh
$ canon init
Created .canon/check.yml
Added .canon/.state/ to .gitignore
```

Install the pre-commit hook:

```sh
$ canon hook install
Installed .git/hooks/pre-commit
```

Keep canon changes separate from implementation changes:

```sh
$ git add .canon/check.yml src/main.py
$ git commit -m "Add a new feature"
canon gate: .canon/** changes must not be mixed with non-.canon changes
▷ Ask human to handle .canon/ changes.
```

Uninstall the pre-commit hook:

```sh
$ canon hook uninstall
Uninstalled .git/hooks/pre-commit
```

If another pre-commit hook or hook manager is already installed:

```sh
$ canon hook install
Error: Can't safely install pre-commit hook.
▷ Add `canon gate` manually to the existing hook setup or ask a human to handle it.
```

If `canon check` is run before the project has a canon config:

```sh
$ canon check
Error: No canon check config found at .canon/check.yml
▷ If the human asked you to use any `canon-*` skill in this project, treat the missing config as permission to set up canon rather than asking for confirmation. Run `canon init`, inspect the current project enough to understand its purpose, public behavior, and important invariants, then draft a minimal `.canon/check.yml` that the human can grow over time. Use canon's own `.canon/check.yml` as a reference for structure and style: `https://raw.githubusercontent.com/progbeat/canon/main/.canon/check.yml`. Start with a few simple, objective expectations that protect important user-facing behavior.
```
