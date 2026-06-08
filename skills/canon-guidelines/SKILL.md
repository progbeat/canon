---
name: canon-guidelines
description: Guidelines for writing maintainable canon expectations
---

# Canon Guidelines

Canon intent comes from the human; these guidelines only describe what makes canon text maintainable.

## Good Canon

- Is self-consistent.
- Leaves freedom for future implementation choices.
- Specifies the necessary minimum and relies on implementers' common sense wherever the canon is silent.
- Focuses on what must be true, rather than exhaustively listing what must not be true.
- Avoids enumerating specific cases when a general statement would suffice.
- Defines each reusable concept once as a highlighted term using bold Markdown.
- Never reintroduces the same concept in multiple expectations.
- Keeps expectations isolated, so each expectation can be edited without having to fix other expectations (unless terms change).

## Bad Canon

- Treats every possible omission as something that must be explicitly specified.
- Specifies both the required behavior and unnecessary negative cases.
- Enumerates specific cases instead of making a general statement.
- Has unclear scope or applies more broadly than intended.
- Duplicates the same assertion in multiple expectations.
- Introduces near-synonyms for the same concept.

## Term Discipline

- A term definition should be self-contained enough to understand the term's meaning.
- Other expectations should use a term as an interface and should not repeat its definition.

## Review Process

- For canon review requests, review only the canon text yourself. Do not review any other project files.
- Do not treat underspecification or canon silence as a finding; report only written text that creates a concrete maintainability issue under these guidelines.
- After drafting findings, spawn a subagent to re-check those findings against these guidelines using this exact prompt template:
  ```
  Review these drafted findings against the $canon-guidelines:

  <findings>

  ! Only check whether these findings are valid under the guidelines.
  ! Do not spawn subagents.
  ```

## Review Questions

- Is this assertion already stated elsewhere?
- Is a new term being introduced where an existing term should be reused?
- Would this wording make a future canon change harder?
- Is this assertion's intended scope clear?
- Could this be stated as a general rule instead of a list of cases?
