# Releasing Rshot

Release from `main` after the changes and the GitHub Actions release workflow have landed.

1. Set `[workspace.package].version` in the root `Cargo.toml` to the release version, without the `v` prefix. Run `cargo update --workspace` to refresh `Cargo.lock` and confirm that `rshot-core`, `rshot-cli`, and `rshot-gui` all have the same version as the planned tag. Commit any version and lockfile changes together after `just check` passes.
2. Preview the notes with `just changelog`. Cocogitto reports and skips the first two commits (`96004f8` and `d2f9a3d`) because their messages predate Conventional Commits.
3. Run `just bump --version X.Y.Z` from a clean worktree. Cocogitto updates `CHANGELOG.md`, creates the release commit, and tags it `vX.Y.Z`. Use the same `X.Y.Z` as the workspace version.
4. Push `main`, then push the new tag. The tag push triggers the release workflow, which builds the macOS archives and attaches them to the GitHub Release.
