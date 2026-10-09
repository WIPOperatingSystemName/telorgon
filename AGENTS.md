# Telorgon Development Agent Guidance

## Objective

Prioritize sound architecture, useful implementation, and efficient progress. Spend effort on
understanding ownership and making coherent changes. Verification should answer specific questions,
not become a ritual after every edit. The user performs much of the interactive and hardware testing.

## Design authority and focused reading

- Use [DEVELOPMENT_SPECIFICATION.md](DEVELOPMENT_SPECIFICATION.md) as the primary general design
  standard. Read it once when first working in this repository; revisit only relevant sections.
- The specification is a target, not a claim about existing implementation. Use the actual code,
  relevant tests, and observed behavior to establish what currently exists.
- Follow existing subsystem contracts unless the task requires changing them. Inspect the relevant
  interfaces and invariants rather than relying on historical design descriptions.
- Search for symbols and inspect focused file ranges before reading large files or whole directories.
  Reuse information already established in the conversation. Avoid repeating unchanged searches.
- Consult specifications or adjacent reference implementations when a concrete uncertainty warrants
  it. Do not perform a multi-project reference audit for every routine change. Adjacent repositories
  are reference material, not authorized edit targets or dependencies.

## Architecture before edits

For a nontrivial change, identify the owning subsystem, public API impact, state/resource ownership,
dependency direction, and affected entry points. Keep this reasoning brief; write a separate design
document only when requested or when a lasting architectural decision needs to be recorded.

- Keep public API, capability implementation, platform/protocol adapters, and host assembly distinct.
- Preserve portable behavior across application, shell, and embedded entry points. Isolate native
  types and feature gates at the appropriate boundaries.
- Prefer focused types and explicit interfaces over global access, oversized managers, or speculative
  abstractions. Give new behavior one canonical owner.
- Keep handwritten files below 1,000 lines; review responsibility boundaries before they approach
  that size. Follow the specification's narrow exception policy for cohesive declarations.
- In existing oversized files, extract the relevant responsibility when practical. Do not launch an
  unrelated rewrite, mechanically split files, or silently expand a monolith.
- Use the existing layout until a migration is in scope. Do not create the entire proposed directory
  tree or duplicate an implementation just to match the specification.
- Make code easy to document through names, types, visibility, and module structure. Add comments for
  non-obvious contracts and safety invariants; do not require comprehensive rustdoc for every change.

## Efficient implementation workflow

- Inspect the working tree and preserve unrelated user changes.
- Read the affected implementation and its direct callers, then complete a coherent change before
  running checks. Do not compile or test after each small edit.
- Reuse established patterns and dependencies where suitable. Avoid unrelated cleanup, broad
  formatting churn, new tooling, and unnecessary abstractions.
- Batch independent reads and searches. Keep tool output focused and summarize lengthy results.
- Proceed with routine implementation decisions within the requested scope. Ask only when unresolved
  ambiguity materially affects behavior, compatibility, ownership, or scope.
- Do not delegate by default. Use sub-agents only when the user requests delegation.

## Proportionate verification

Choose the smallest check that can resolve a concrete uncertainty. Testing effort should follow the
risk and behavior changed, not the number of files or prompts. Existing evidence remains useful
until a relevant change invalidates it.

| Change | Default verification |
| --- | --- |
| Documentation, instructions, comments | Inspect the diff and affected links or structure; no Cargo checks |
| Simple local edit or mechanical move | Focused inspection; a relevant compile check if types, imports, or visibility changed |
| Deterministic logic or bug fix | Use an existing focused test; add a small regression test when it meaningfully verifies behavior |
| Public API, feature gates, cross-module integration | Narrow compile/test targets covering the changed contract |
| Unsafe code, resource lifetime, synchronization, authorization | Targeted verification of the affected invariant; broaden only when evidence requires it |
| Visual, device, compositor, or media interoperability | Compile or deterministic checks where useful; leave live qualification to the user |

- Do not write tests that merely mirror the implementation, verify trivial assignments, or exist only
  to accompany every addition. Prefer tests with observable outcomes and meaningful failure cases.
- A small deterministic test is encouraged when it can cheaply establish functionality or reproduce
  a bug. Do not build an elaborate test harness for a minor change.
- Do not run full workspace tests, exhaustive feature matrices, all-target linting, benchmarks, or
  repeated builds by default. Use them when requested or when a specific unresolved risk warrants it.
- Once relevant checks pass, stop checking unless subsequent changes invalidate that evidence.
- If a check is blocked by missing native dependencies or environment setup, report the limitation.
  Do not spend the task installing toolchains or rebuilding the environment unless that is in scope.
- Never claim compilation proves runtime behavior, or report unrun checks as passing. State material
  verification limits plainly without treating every untested hardware path as a blocker.

## User-run testing and scope

Leave GUI applications, compositor sessions, device interaction, services, and background processes
to the user unless explicitly asked to run them. When useful, provide a short manual check describing
the changed behavior and expected result. Avoid exhaustive testing checklists unless requested.

Do not modify system configuration, install dependencies globally, or change adjacent repositories
as incidental setup. Keep generated work and build outputs in their intended locations.

## Documentation and handoff

- Do not recreate the removed documentation directory or generate a replacement documentation tree.
  Keep general development guidance in the root specification and agent instructions. Use source
  comments and rustdoc for local contracts when needed; add standalone documents only when requested.
- Do not spend time writing or updating documentation unless the user explicitly requests it.
  Do not generate plans, work logs, status documents, or comprehensive rustdoc as incidental work.
  Keep only concise code-local comments needed to explain safety invariants or non-obvious behavior.
  If a code change makes existing documentation materially misleading, mention it briefly in the
  handoff rather than starting an unrequested documentation task.
- Keep progress updates brief and focused on findings, architectural decisions, and actual blockers.
- The final response should summarize the change, relevant verification, and material limitations.
  Include file links where useful; avoid a transcript of commands or routine reasoning.
- After repository edits, include a short suggested commit message. Do not commit unless requested.

## Commit and pull request handoff

Read the [shared organization contribution instructions](https://github.com/WIPOperatingSystemName/.github/blob/main/AGENTS.md)
before the final handoff. In the standard contributor workspace, the local
copy is `~/wip-os/.github/AGENTS.md`. If neither copy is available, follow this
minimum workflow:

- After completing requested edits and relevant verification, summarize the
  changes and suggest a commit message. Ask once whether the user wants a
  commit and PR to `WIPOperatingSystemName/telorgon`'s default branch.
- Honor explicit submission authorization or a prior decline without asking
  again. Otherwise wait for approval before committing, pushing or opening
  a PR; leave the changes local if the user declines.
- Submit only intended files on a feature branch, push to the contributor's
  personal fork and target the organization repository's default branch.
  Keep component changes in their owning repository. Do not merge the PR
  or update distro submodule pins without separate authorization.
