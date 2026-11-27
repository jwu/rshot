# Rshot engineering conventions

## Workflow

- The lead agent communicates with the user, clarifies requirements, designs the approach, delegates work, reviews changes, and delivers the result. Delegate all work that can be delegated and keep user communication responsive while subagents work.
- Use `hc_subagent` to delegate implementation to a `coder` or `worker` and read-only investigation to a `scout`.
- Do each task in its own worktree at `~/worktrees/<project-name>/<task-name>`, using the task name as the branch name. For example, run `just wt-new feat/capture` from `main`.
- The lead reviews the changes and confirms `just check` passes before merging. After the merge, remove the task worktree with `just wt-rm feat/capture` from another worktree.

## Commits and releases

- Run `just hooks` to install the Cocogitto `commit-msg` hook. Use `just commit feat "add capture command" cli` to commit staged changes.
- Write English Conventional Commits in the form `type(scope): subject`. Use one of `core`, `cli`, `gui`, `deps`, `ci`, `docs`, or `release` as the scope. Repository-wide changes may omit the scope.
- Write the subject in the imperative mood, start with a lowercase letter, and omit the trailing period. Keep the subject to 72 characters or fewer.
- Keep each commit to one logical change. Run `just check` and commit code only after it passes.
- Preview release notes with `just changelog`. Use `just bump --auto --dry-run` to preview the next version and `just bump` to create the changelog commit and version tag. Use `just bump --version <version>` to set a release version explicitly. Cargo package versions are maintained separately.

## Code style

- Use English for code, identifiers, documentation, and commit messages.
- Prefer self-explanatory names and omit comments by default. Comment only when the code cannot convey a non-obvious reason, a platform quirk, or an `unsafe` invariant. Remove commented-out code.
- Use `thiserror` for library errors and `anyhow` at binary boundaries as the workspace takes shape. Library code must avoid `unwrap` and `expect` outside tests.

## Testing and checks

- Add tests for every feature and bug fix. Use golden images or pixel assertions for rendering behavior.
- Run `just check` before committing and merging. CI runs the same formatting, lint, and test checks. The test recipe uses `cargo-nextest` when installed and falls back to `cargo test`.

## Project layout

The project is migrating from a single crate to this target workspace layout.

- `crates/rshot-core` handles screenshot capture, annotation models, and rendering.
- `crates/rshot-cli` serves AI agents. It produces PNG images and JSON metadata, keeps standard output machine-readable, sends diagnostics to standard error, and exits with a nonzero status on failure.
- `crates/rshot-gui` provides the graphical interface.
