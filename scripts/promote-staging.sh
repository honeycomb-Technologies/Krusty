#!/bin/sh
# Open (or reuse) the PR that promotes the current staging branch onto main.
# Does not merge. Main stays review-protected.
#
# Usage: sh scripts/promote-staging.sh [codex/release-staging-YYYYMMDD]
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"
staging=${1:-codex/release-staging-20260801}
repo=${GITHUB_REPOSITORY:-honeycomb-Technologies/Mitsuro}

git fetch origin \
  "refs/heads/main:refs/remotes/origin/main" \
  "refs/heads/$staging:refs/remotes/origin/$staging"
ahead=$(git rev-list --count "origin/main..origin/${staging}")
behind=$(git rev-list --count "origin/${staging}..origin/main")
echo "staging ${staging} is ${ahead} commit(s) ahead of main, ${behind} behind"

if [ "$ahead" = "0" ]; then
  echo "Nothing to promote. If you still need binaries, run: sh scripts/release-status.sh"
  exit 0
fi

existing=$(gh pr list --repo "$repo" --base main --head "$staging" --state open --json number,url --jq '.[0].url // empty')
if [ -n "$existing" ]; then
  echo "Promote PR already open: $existing"
  exit 0
fi

gh pr create --repo "$repo" --base main --head "$staging" \
  --title "Promote ${staging} to main" \
  --body "$(cat <<EOF
## Outcome

Promote the current staging tip onto \`main\` so the next cut can version, tag, and publish binaries.

## What changed

Staging \`${staging}\` is ${ahead} commit(s) ahead of \`main\`.

## Validation

- [ ] CI on this PR
- [ ] After merge: Version (Sampo) opens or publishes if changesets exist
- [ ] Release binaries produces GitHub Release assets
- [ ] Honey uses \`sh scripts/honey-upgrade.sh v{version}\` only after the linux archive exists

## Compatibility

Does not tag or restart Honey by itself.
EOF
)"
