set positional-arguments
set working-directory := "../.."

[default]
[private]
default:
    @just --list wt

# Create or reuse a task branch worktree from the given base.
new BRANCH BASE="main":
    @python3 scripts/worktree.py new "$@"

# List registered worktrees by name, branch, HEAD, and absolute path.
list:
    @python3 scripts/worktree.py list

# Print the absolute path of a managed worktree.
path NAME:
    @python3 scripts/worktree.py path "$@"

# Run the project's formatting, lint, and test checks in a worktree.
check NAME:
    @python3 scripts/worktree.py check "$@"

# Run cargo test --workspace in a worktree, forwarding extra arguments.
test NAME *ARGS:
    @python3 scripts/worktree.py test "$@"

# Remove a clean worktree and keep its local branch.
remove NAME:
    @python3 scripts/worktree.py remove "$@"
