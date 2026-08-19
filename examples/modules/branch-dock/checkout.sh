#!/bin/sh
# Check out the branch whose dock row was clicked.
#
# A clicked dock row invokes its action with the row's identity in the env:
#   LUVUS_MODULE_ROW_VALUE  the row's `value` (here: the branch name)
#   LUVUS_MODULE_ROW_TEXT   the row's visible text
#   LUVUS_MODULE_ROW_INDEX  its position in the dock
set -eu

luvus="${LUVUS_BIN_PATH:-luvus}"
repo="${LUVUS_WORKSPACE_CWD:-$PWD}"
branch="${LUVUS_MODULE_ROW_VALUE:-}"

if [ -z "$branch" ]; then
  "$luvus" ui toast "no branch selected"
  exit 1
fi

if git -C "$repo" checkout "$branch" >/dev/null 2>&1; then
  "$luvus" ui toast "switched to $branch"
  sh "$(dirname "$0")/refresh.sh"   # repaint so the current-branch dot moves
else
  # A dirty tree is the usual reason, so say so rather than failing silently.
  "$luvus" ui toast "cannot switch to $branch (uncommitted changes?)"
  exit 1
fi
