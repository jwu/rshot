repository := `dirname "$(git rev-parse --path-format=absolute --git-common-dir)"`

# Create, inspect, check, and remove worktrees.
mod wt "just/wt"

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

[positional-arguments]
changelog *args:
    cog changelog "$@"

[positional-arguments]
bump mode="--auto" *args:
    cog bump "$@"
