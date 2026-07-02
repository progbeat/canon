---
name: canon-conflict
description: Validate alleged canon conflicts before reporting them. Use when canon text appears contradictory or impossible to satisfy together.
---

# Canon Conflict

A **canon conflict** is a logical inconsistency between canon expectations: no implementation can satisfy all cited canon text at the same time.

Use only files under `.canon/` as conflict evidence.

Do not use implementation files, tests, runtime behavior, logs, or current behavior as conflict evidence.

Do not treat underspecification, unclear wording, maintainability issues, missing implementation support, failing tests, or current implementation behavior as a canon conflict.

Draft the conflict with precise `.canon/` file references and explain why the cited text cannot be satisfied together.

After drafting a conflict, spawn a subagent to re-check it using this exact prompt template:

```
Review this drafted canon conflict against the $canon-guidelines using only files under `.canon/`:

<conflict>

! Only check whether the cited canon text logically implies that no implementation can satisfy all cited expectations at the same time.
! Do not inspect implementation files, tests, runtime behavior, or logs.
! Do not spawn subagents.
```

Do not report a canon conflict unless the subagent validates it.

If the subagent does not validate it, treat the issue as something other than a canon conflict and continue with the applicable workflow.
