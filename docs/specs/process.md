# Development Process Specification

## Data Source Abstraction

All data interfaces (hardware templates, logical devices, services, config templates) must be designed behind traits/abstractions that allow substitution or addition of alternative data sources in the future (e.g., databases, APIs, remote storage). The initial implementation uses filesystem-based sources, but the interface boundary must not assume filesystem access.

## Development Methodology

### Red-Green TDD

All implementation is driven by red-green Test-Driven Development:
1. Write a failing test (red).
2. Write the minimal code to make it pass (green).
3. Refactor if needed.

TDD implementation work is performed by Sonnet models.

### Commit Discipline

- Commits are made frequently, at each point where all current tests pass.
- Each commit represents a stable, green state — no commits with failing tests.

### Code Review Cycle

After each commit:
1. A fresh independent Opus model reviews the commit.
2. The main model reviews the commit.
3. If review comments require changes, those changes are implemented via TDD (red-green cycle) by Sonnet.
4. The cycle repeats until there are no outstanding review comments.
