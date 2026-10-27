#!/usr/bin/env bash
# Lists what changed on the /dev/null server since docs/api-contract.lock, in the files devnull depends on.
#
#   scripts/check-api.sh           exit 1 and print the diff when any of them changed
#   scripts/check-api.sh --update  after devnull is fixed, pin the server's current main instead
#
# Set DEVNULL_SERVER_REPO to check a local clone or another remote.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
lock="$root/docs/api-contract.lock"
repo="${DEVNULL_SERVER_REPO:-https://github.com/dev-null-sh/dev-null.sh}"

pinned="$(awk '$1 == "commit" { print $2 }' "$lock")"
mapfile -t files < <(awk '$1 == "file" { print $2 }' "$lock")

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
# Without file contents, only the history, so the diff can reach the pinned commit and fetch just the files it needs.
git clone --quiet --filter=blob:none --no-checkout "$repo" "$work/server"
cd "$work/server"
head="$(git rev-parse HEAD)"

if [ "${1:-}" = "--update" ]; then
  sed -i.bak "s/^commit .*/commit $head/" "$lock" && rm -f "$lock.bak"
  echo "Pinned the server at $head. Update the date and commit in docs/api-contract.md too."
  exit 0
fi

if ! git cat-file -e "$pinned^{commit}" 2>/dev/null; then
  echo "The pinned server commit $pinned is gone from $repo. Check the contract by hand, then run with --update." >&2
  exit 2
fi

if git diff --quiet "$pinned" "$head" -- "${files[@]}"; then
  echo "No changes in the server files devnull depends on (pinned ${pinned:0:7}, server at ${head:0:7})."
  exit 0
fi

echo "The server changed files devnull depends on since ${pinned:0:7} (now ${head:0:7}):"
echo
git diff --stat "$pinned" "$head" -- "${files[@]}"
echo
echo "Commits that touched them:"
git log --oneline "$pinned..$head" -- "${files[@]}"
echo
git diff "$pinned" "$head" -- "${files[@]}"
echo
echo "Compare with docs/api-contract.md and src/model.rs, fix what moved, then run scripts/check-api.sh --update."
exit 1
