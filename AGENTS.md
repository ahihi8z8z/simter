# AGENTS.md — Engineering instructions for coding agents

## 1. Authority and orientation

- Read `spec.md` before planning or editing. It defines product behavior, scope, non-goals, and acceptance criteria. This file defines the implementation workflow, not additional product requirements.
- Read `README.md`, existing architecture notes, the build/test configuration, and relevant code before modifying an area. Do not assume the repository uses a particular framework or test runner.
- In a conflict, follow higher-priority user instructions; otherwise treat `spec.md` as the source of truth for product behavior. Surface contradictions instead of silently resolving them by changing scope.
- Preserve working behavior. Prefer the smallest coherent change to a rewrite.

## 2. Project-specific boundaries

For Simter (Simple Terminal), the SSH app described in `spec.md`:

- Windows desktop app, with SSH host discovery, embedded interactive terminal, and adjacent SFTP file browser.
- Keep the OS OpenSSH client and existing SSH configuration as the authoritative connection/authentication implementation. Do not implement an SSH protocol stack, a credential vault, or a second independent host database.
- Keep Byobu/tmux on the remote host responsible for persistent shell sessions. Do not treat reopened application tabs as proof of remote process persistence.
- Preserve terminal input, escape sequences, resize behavior, and Byobu function keys; avoid global shortcuts that intercept F2–F9 while the terminal has focus.
- Never silently rewrite complex SSH config (`Include`, `Match`, wildcard `Host` blocks). Preserve comments and unrelated directives.
- Do not add features, dependencies, background services, telemetry, or architectural layers outside the spec without a concrete requirement.

When reusing this file in another project, replace this section with that project's specific invariants.

## 3. Work in small, verifiable slices

1. Inspect `git status`, relevant files, tests, and recent conventions before changing code. Preserve any existing user changes.
2. For a non-trivial task, identify the desired behavior, affected components, risks, verification strategy, and **the next smallest independently verifiable slice**. A brief plan is sufficient; do not produce elaborate plans for trivial edits.
3. Implement one slice at a time: a focused behavior, bug fix, refactor, or enabling test/infrastructure change.
4. Add or update the most relevant tests, then run focused checks. Inspect the diff for accidental changes.
5. Commit a completed slice only when its required checks pass, or explicitly record an environmental limitation and keep the unverified work uncommitted unless the user authorizes otherwise.
6. Continue with the next slice. Keep the application in a buildable state at each commit, whenever the project's toolchain is available.

An atomic commit is one coherent reason to change the code, **not** one file per commit or an arbitrary line limit. A small feature may legitimately touch implementation, tests, and documentation in the same commit. Separate unrelated work.

## 4. Git and commit discipline

- Use **Conventional Commits 1.0.0**: `type(scope): imperative summary` where scope is optional.
- Common types: `feat`, `fix`, `refactor`, `test`, `docs`, `build`, `ci`, `chore`, `perf`.
- Examples:
  - `feat(hosts): list explicit SSH host aliases`
  - `test(terminal): cover function-key forwarding`
  - `fix(sftp): preserve remote paths with spaces`
  - `refactor(ssh): isolate config resolution`
- Keep the subject concise and accurately scoped. Explain *why* in the body when the reason is not obvious. Mark breaking changes according to Conventional Commits.
- Do not mix refactoring with new behavior unless inseparable. Do not bundle formatting-only changes into functional commits.
- Stage intentional paths or hunks (`git add <path>` or `git add -p`); inspect `git diff --cached` before every commit. Never blindly stage everything in a dirty worktree.
- Do not amend, rebase, reset, force-push, delete branches, or revert user work without explicit authorization.
- Do not commit secrets, private keys, host credentials, real environment config, temporary artifacts, or generated build outputs.
- Commit locally after each verified slice when working in an agent-owned task branch or when the user has explicitly asked for commits. Otherwise prepare commit-ready slices and request authorization before mutating shared history. Never push without an explicit request.

## 5. Design and code quality

- Prefer simple, explicit boundaries. Separate UI state, host discovery/config resolution, terminal/process lifecycle, and file-transfer operations, without speculative abstractions.
- Reuse maintained libraries where they provide difficult platform capabilities (e.g., terminal rendering/ConPTY). Document material dependency and security trade-offs.
- Preserve error context. Distinguish invalid configuration, authentication failure, unreachable host, missing remote capability, transfer failure, and user cancellation.
- Handle process shutdown, cancellation, stream errors, file paths with spaces/Unicode, and GUI responsiveness. Avoid blocking the UI thread with network or subprocess I/O.
- Do not silently swallow errors, introduce magic fallbacks, or weaken validation/tests to make a check pass.
- Keep changes readable and consistent with existing project conventions; avoid premature optimization and unrelated cleanup.

## 6. Verification and testbed policy

- For each change, select the lowest-cost tests that actually exercise the modified behavior. Add regression tests for bug fixes when feasible.
- Run existing formatter/linter, static/type checks, unit/integration tests, and build commands that apply to affected code. Derive exact commands from the repository; do not invent successful test results.
- Use mocks/fakes for fast tests at process and network boundaries, but **never claim mocks prove real SSH, SFTP, terminal, or Windows PTY compatibility**.
- Before declaring an end-to-end feature accepted, test it on a real **Windows 10/11 client + Linux SSH server** testbed. For relevant changes, cover SSH config resolution (`Host`, `Include`, jump hosts), authentication, embedded terminal resize, Byobu F2–F9, SFTP transfers, errors/cancellation, and reconnect behavior as specified.
- If the testbed or prerequisites are unavailable, explicitly report `NOT TESTED ON TESTBED`, which behaviors remain unverified, and how to test them. Do not invent results or claim feature acceptance.
- Never use real user credentials or production servers as test fixtures unless explicitly authorized. Prefer disposable test identities and non-production hosts.

## 7. Definition of done for a slice

A slice is ready to commit when:

- Its purpose is clear and within `spec.md`; unrelated work is excluded.
- Behavior is implemented with relevant tests; affected checks pass.
- The diff has been reviewed for security, compatibility, unintended config edits, and secrets.
- Documentation is updated if user-visible behavior, setup, or an architectural decision changed.
- The commit message accurately describes the change, and the working tree's unrelated changes remain untouched.

A slice requiring a real testbed may be committed as verified implementation work **only if** its automated checks pass and its unverified end-to-end status is stated explicitly; it must not be reported as accepted or complete against the spec before testbed validation. If the current task requires testbed acceptance as a commit gate, leave it uncommitted until tested.

## 8. Communication and escalation

- Report progress by completed slices/commits, not by vague percentages.
- On completion, report: what changed; commit hashes/messages (if any); checks actually run and outcomes; testbed status; remaining gaps or blockers.
- If a requirement is ambiguous and materially affects architecture, security, user data, or acceptance behavior, state the precise decision needed. For reversible implementation details, choose the simplest spec-compliant option and document the choice.
- Never claim an operation, test, commit, or deployment succeeded without observing its result.

## 9. Maintain these instructions

- Replace generic guidance with exact repository commands and paths once the stack is established; keep this file short and actionable.
- Record substantial architecture decisions in a separate ADR or design note, not as sprawling rules here.
- Add nested `AGENTS.md` files only when a subdirectory truly has distinct build or verification constraints.
