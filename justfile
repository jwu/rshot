repository := `dirname "$(git rev-parse --path-format=absolute --git-common-dir)"`
project := file_name(repository)

# List available recipes.
default:
    @just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    if cargo nextest --version >/dev/null 2>&1; then cargo nextest run --workspace; else cargo test --workspace; fi

check: fmt-check lint test

hooks:
    cd {{quote(repository)}} && cog --config {{quote(justfile_directory() + "/cog.toml")}} install-hook commit-msg

# Commit staged changes using a Conventional Commit type, subject, and optional scope.
commit type subject scope="":
    cog commit -- {{quote(type)}} {{quote(subject)}} {{if scope == "" { "" } else { quote(scope) }}}

# Create a worktree and a branch with the given name.
wt-new name:
    git check-ref-format {{quote("refs/heads/" + name)}}
    git worktree add -b {{quote(name)}} "$HOME/worktrees/"{{quote(project + "/" + name)}}

# Remove a clean worktree by branch name.
wt-rm name:
    git check-ref-format {{quote("refs/heads/" + name)}}
    git worktree remove "$HOME/worktrees/"{{quote(project + "/" + name)}}

[positional-arguments]
changelog *args:
    cog changelog "$@"

[positional-arguments]
bump mode="--auto" *args:
    cog bump "$@"
