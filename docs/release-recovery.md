# Release recovery

Presswerk's GitHub Actions workflow prepares releases from successful pushes to `main`, then
dispatches CI for the exact generated release commit. Publication happens only after that commit
passes the same flake checks.

## Failed preparation

If a normal `main` run fails before it pushes a release commit, fix the failure or rerun that
workflow. A stale run stops when `main` no longer points to the commit that started it; the newer
push owns the next preparation attempt.

## Failed candidate checks

A release commit that fails its checks must not be published. Revert that untagged release commit
on `main`, fix the failure, and let the next successful push prepare a fresh release commit. Do
not add fixup commits on top of the pending release version.

## Partial or failed publication

Rerun the candidate workflow for the exact release commit. `presswerk publish` keeps matching
state and completes missing state, so a retry can reuse an existing matching tag or GitHub
Release draft safely.

Do not move or overwrite a conflicting tag, and do not delete published release state. Inspect
and resolve conflicts manually before allowing later releases to continue.
