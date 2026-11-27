import subprocess
import sys
from pathlib import Path


class WorktreeError(Exception):
    pass


def git(directory, *args):
    return subprocess.check_output(["git", "-C", directory, *args], text=True).rstrip("\n")


def worktrees(repo):
    output = git(repo, "worktree", "list", "--porcelain", "-z")
    entries = {}
    for record in output.rstrip("\0").split("\0\0"):
        fields = {}
        for field in record.split("\0"):
            key, _, value = field.partition(" ")
            fields[key] = value
        entries[Path(fields["worktree"])] = fields
    return entries


def branch_name(entry):
    return entry.get("branch", "").removeprefix("refs/heads/")


def directory_name(branch):
    return branch.replace("/", "-")


def managed_name(root, path, entry):
    branch = branch_name(entry)
    if branch and path.parent == root and path.name == directory_name(branch):
        return path.name
    return None


def managed_worktree(repo, root, entries, name):
    if not name or name in (".", "..") or "/" in name:
        raise WorktreeError(f"Expected a worktree directory name, got {name!r}.")

    path = root / name
    entry = entries.get(path)
    if entry is None or managed_name(root, path, entry) != name:
        raise WorktreeError(f"No managed worktree named {name!r} at {path}.")
    if path.is_symlink() or not path.is_dir() or "prunable" in entry:
        raise WorktreeError(f"Worktree is unavailable at {path}.")
    if Path(git(path, "rev-parse", "--show-toplevel")) != path:
        raise WorktreeError(f"Git worktree root differs from {path}.")
    common_dir = Path(git(path, "rev-parse", "--path-format=absolute", "--git-common-dir"))
    if common_dir.resolve() != (repo / ".git").resolve():
        raise WorktreeError(f"Worktree at {path} belongs to another repository.")
    git_dir = Path(git(path, "rev-parse", "--path-format=absolute", "--git-dir"))
    backlink = git_dir / "gitdir"
    if not backlink.is_file() or (git_dir / backlink.read_text().rstrip("\n")).resolve() != path / ".git":
        raise WorktreeError(f"Git registration does not match the worktree at {path}.")
    return path


def create_worktree(repo, root, entries, branch, base):
    if git(repo, "check-ref-format", "--branch", branch) != branch:
        raise WorktreeError(f"Expected a local branch name, got {branch!r}.")
    path = root / directory_name(branch)
    entry = entries.get(path)
    if entry is not None and branch_name(entry) != branch:
        registered_branch = branch_name(entry) or "detached HEAD"
        raise WorktreeError(f"Worktree directory {path} is registered to {registered_branch}.")
    if entry is None and (path.exists() or path.is_symlink()):
        raise WorktreeError(f"Path already exists at {path} without a worktree registration.")

    base_commit = git(Path.cwd(), "rev-parse", "--verify", "--end-of-options", f"{base}^{{commit}}")
    branch_ref = f"refs/heads/{branch}"
    branch_exists = subprocess.run(
        ["git", "-C", repo, "show-ref", "--verify", "--quiet", branch_ref]
    ).returncode == 0
    if branch_exists and subprocess.run(
        ["git", "-C", repo, "merge-base", "--is-ancestor", base_commit, branch_ref]
    ).returncode != 0:
        raise WorktreeError(f"Branch {branch!r} does not contain base {base!r}.")
    if entry is not None:
        return managed_worktree(repo, root, entries, path.name)
    if branch_exists:
        git(repo, "worktree", "add", "--", path, branch)
    else:
        git(repo, "worktree", "add", "-b", branch, "--", path, base_commit)
    return path


def main():
    command, *args = sys.argv[1:]
    repo = Path(git(Path.cwd(), "rev-parse", "--path-format=absolute", "--git-common-dir")).parent
    root = (Path.home() / "worktrees" / repo.name).resolve()
    entries = worktrees(repo)

    if command == "list":
        for path, entry in entries.items():
            name = managed_name(root, path, entry) or "-"
            branch = branch_name(entry) or "detached"
            print("\t".join((name, branch, entry.get("HEAD", ""), str(path))))
        return 0
    if command == "new":
        print(create_worktree(repo, root, entries, *args))
        return 0

    name, *extra = args
    path = managed_worktree(repo, root, entries, name)
    if command == "path":
        print(path)
    elif command == "remove":
        status = git(path, "status", "--porcelain=v1", "--untracked-files=all", "--ignored=matching")
        if status:
            raise WorktreeError(f"Worktree at {path} has changes or local files:\n{status}")
        git(repo, "worktree", "remove", "--", path)
        print(f"Removed {path}. Kept branch {branch_name(entries[path])!r}.")
    elif command == "check":
        justfile = Path(__file__).resolve().parents[1] / "justfile"
        return subprocess.call(["just", "--justfile", justfile, "--working-directory", path, "check"])
    elif command == "test":
        return subprocess.call(["cargo", "test", "--workspace", *extra], cwd=path)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (WorktreeError, OSError) as error:
        sys.exit(str(error))
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)
