# Guidelines for AI Agents

## Principles

**Order of operations**

1. **Spec first**: the spec is the durable source of truth. Code can be deleted and rewritten; the spec survives.
2. **Tests next**: test coverage is from day one, not added later. Only test code that has meaningful logic (branching, transformations, error handling). Don't test code that can only break if the language, runtime, or a dependency breaks.
3. **Code last**: write code along with tests, implementing spec in full.

**Drift rule**: resolve drift by propagating changes upward in this order: `code -> execution plan -> implementation spec -> design spec`.
If code/tests reveal missing or incorrect requirements, the agent must report the drift to the user and wait for approval before updating affected layers in that sequence.

## Specification System

This repo uses two persistent spec types and one transient execution activity.

### 1) Design Specs (`/docs/design/`)

Purpose: define what the system must do.

Design specs contain:

- User-visible behavior and contracts.
- Scope, non-goals, and acceptance criteria.
- Architecture constraints that should remain true across rewrites.

Design specs do not contain:

- Library/framework/package choices.
- File paths, type/function signatures, or symbol names.
- Internal algorithms or step-by-step execution mechanics unless externally observable.

### 2) Implementation Specs (`/docs/implementation/`)

Purpose: define how a feature is implemented in durable engineering terms.

Implementation specs contain:

- Concrete dependency choices and rationale.
- Interfaces, invariants, constraints, and error contracts.
- Internal behavior that is not user-facing.
- Required tests for meaningful logic.
- Structural boundaries only when they are durable architectural constraints; include rationale when enforced.

Implementation specs do not contain:

- One-time task sequencing, timelines, or PR choreography.
- Incidental file/package/directory layout that can change without violating the spec contracts.

Unless explicitly declared as an architectural constraint, code organization is refactorable.

### 3) Execution Plan (Transient, Not a Document Type)

A phased plan is required before implementation work begins, but it is a conversation artifact, not a committed spec artifact.
When implementing a doc from `/docs/implementation/`, the agent must follow this flow:

1. Propose a phased breakdown to the user.
2. Wait for user review and an explicit start instruction before making implementation edits or commits.
3. Keep each phase atomic — producing a coherent, compilable, runnable product — and small enough for human review (ideally under 200 lines changed, excluding lockfiles and generated files).
4. After start, execute phases autonomously: commit after each phase, report status, and continue to the next phase without waiting for additional user confirmation unless blocked by missing input, failures, or detected drift.
5. Reports must focus on decision-relevant content: requirement or behavior now satisfied, scope boundaries that remain unchanged, validation evidence, what is guaranteed correct based on passing tests, assumptions made, risks introduced, any detected drift and proposed updates across execution plan/implementation spec/design spec, and why continuing is safe.

## Boundary Rule

If a statement would become false after swapping implementation dependencies while preserving behavior, it belongs in an implementation spec, not a design spec.

Exception:
A design spec may name a concrete dependency only when that dependency is itself a product requirement (for example compliance, mandated platform/runtime, or explicit interoperability commitment). The reason must be stated explicitly.

## Naming

Spec files are numbered with a zero-padded three-digit prefix starting from `001`, followed by a hyphen and a descriptive slug: `001-slug.md`. New specs take the next available number within their directory.

## Granularity and References

- Design specs may cover multiple features.
- Implementation specs must be small enough to implement in a few phases; split by feature/coherent slice.
- Design docs may reference design docs.
- Implementation docs may reference design and implementation docs.
- Design docs must never reference implementation docs.
- All references use markdown links.

## Language

Specs must be definitive and concise: no TBDs, no unresolved decisions, no optional ambiguity.
Specs must also be internally consistent and coherent: avoid patchy wording, contradictions, and overlapping rules that change meaning, and ensure revisions read as if written that way from the beginning.
Resolve uncertainty with the user before writing it into a spec.

## Coding Quality

- Code changes must be coherent and maintainable; prefer refactoring over layered, patchy fixes.
- Prefer fewer lines when correctness and maintainability are preserved; avoid duplicate logic by relying on existing guarantees from types, called functions, and validated interfaces.
- Directory and package structure should reflect durable boundaries defined by specs, not short-term convenience.
- Adding a dependency is preferred over bespoke code.
- Do not introduce hacky workarounds. If a workaround is unavoidable, pause and get user approval first.

## Misc Rules

- Commit messages follow `type(scope): summary`.
- Always use package manager commands to add or update dependencies.
- `README.md` is not a spec, but it must be updated when it is no longer true due to code change.
- If a user request appears to conflict with these guidelines, explicitly double-check intent with the user before proceeding.