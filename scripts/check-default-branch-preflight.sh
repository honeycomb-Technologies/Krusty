#!/usr/bin/env bash
set -u

# Read-only Mitsuro governance/default-branch preflight.
#
# Verifies that the repository's authoritative default branch, the remote HEAD,
# and the local origin/HEAD tracking symref all agree on main. It also checks
# that main is protected either by classic branch protection or by an active
# GitHub repository ruleset that applies to refs/heads/main.
#
# This script only uses read-only git/gh commands. It does not push, mutate
# GitHub settings, dispatch workflows, create releases, or read secret values.

EXPECTED_DEFAULT_BRANCH="${EXPECTED_DEFAULT_BRANCH:-main}"
REMOTE="${REMOTE:-origin}"
failures=0

info() {
  printf '%s\n' "$*"
}

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  failures=$((failures + 1))
}

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    fail "required command not found: $cmd"
  fi
}

json_value() {
  local expr="$1"
  python3 -c "import json, sys; data=json.load(sys.stdin); value=($expr); print(value if value is not None else '')"
}

ruleset_applies_to_branch() {
  local default_branch="$1"
  local branch="$2"
  python3 -c '
import fnmatch
import json
import sys

def branch_ref_name(branch_name):
    return branch_name if branch_name.startswith("refs/") else f"refs/heads/{branch_name}"

def expand_pattern(pattern, default_branch):
    if pattern == "~DEFAULT_BRANCH":
        return branch_ref_name(default_branch)
    if pattern.startswith("refs/") or pattern == "*":
        return pattern
    return branch_ref_name(pattern)

def pattern_matches(pattern, branch, default_branch):
    branch_ref = branch_ref_name(branch)
    expanded = expand_pattern(pattern, default_branch)
    return expanded == branch_ref or fnmatch.fnmatchcase(branch_ref, expanded)

def applies(ruleset, default_branch, branch):
    if ruleset.get("target") != "branch" or ruleset.get("enforcement") != "active":
        return False

    conditions = ruleset.get("conditions")
    if not conditions:
        name = str(ruleset.get("name") or "").lower()
        branch_l = branch.lower()
        return branch_l in name or (branch == default_branch and "default" in name)

    ref_name = conditions.get("ref_name") or {}
    includes = ref_name.get("include") or []
    excludes = ref_name.get("exclude") or []

    included = True if not includes else any(
        pattern_matches(pattern, branch, default_branch) for pattern in includes
    )
    excluded = any(pattern_matches(pattern, branch, default_branch) for pattern in excludes)
    return included and not excluded

ruleset = json.load(sys.stdin)
sys.exit(0 if applies(ruleset, sys.argv[1], sys.argv[2]) else 1)
' "$default_branch" "$branch"
}

require_command git
require_command gh
require_command python3

if [[ "$failures" -ne 0 ]]; then
  exit 1
fi

repo_json=""
repo_err=""
repo_err_file="$(mktemp)"
if repo_json=$(gh repo view --json nameWithOwner,defaultBranchRef 2>"$repo_err_file"); then
  :
else
  repo_err="$(<"$repo_err_file")"
  rm -f "$repo_err_file"
  fail "unable to read GitHub repository metadata with gh repo view: ${repo_err:-unknown error}"
  info "Default-branch preflight failed with $failures issue(s)."
  exit 1
fi
rm -f "$repo_err_file"

owner_repo="$(printf '%s' "$repo_json" | json_value "data.get('nameWithOwner')")"
github_default_branch="$(printf '%s' "$repo_json" | json_value "(data.get('defaultBranchRef') or {}).get('name')")"

if [[ -z "$owner_repo" ]]; then
  fail "unable to determine GitHub owner/repo from gh repo view"
else
  info "GitHub repository: $owner_repo"
fi

if [[ -z "$github_default_branch" ]]; then
  fail "unable to determine GitHub default branch from gh repo view"
else
  info "GitHub default branch: $github_default_branch"
fi

if [[ -n "$github_default_branch" && "$github_default_branch" != "$EXPECTED_DEFAULT_BRANCH" ]]; then
  fail "GitHub default branch is $github_default_branch, expected $EXPECTED_DEFAULT_BRANCH"
fi

remote_output=""
remote_err_file="$(mktemp)"
if remote_output=$(git ls-remote --symref "$REMOTE" HEAD 2>"$remote_err_file"); then
  :
else
  remote_err="$(<"$remote_err_file")"
  rm -f "$remote_err_file"
  fail "unable to read remote $REMOTE HEAD with git ls-remote --symref: ${remote_err:-unknown error}"
  remote_output=""
fi
rm -f "$remote_err_file"

remote_head_branch=""
if [[ -n "$remote_output" ]]; then
  while IFS= read -r line; do
    if [[ "$line" =~ ^ref:[[:space:]]+refs/heads/(.+)[[:space:]]+HEAD$ ]]; then
      remote_head_branch="${BASH_REMATCH[1]}"
      break
    fi
  done <<<"$remote_output"
fi

if [[ -z "$remote_head_branch" ]]; then
  fail "unable to parse remote $REMOTE HEAD symref from git ls-remote output"
else
  info "remote $REMOTE HEAD: $remote_head_branch"
fi

if [[ -n "$remote_head_branch" && -n "$github_default_branch" && "$remote_head_branch" != "$github_default_branch" ]]; then
  fail "remote $REMOTE HEAD points to $remote_head_branch, expected $github_default_branch"
elif [[ -n "$remote_head_branch" && -z "$github_default_branch" && "$remote_head_branch" != "$EXPECTED_DEFAULT_BRANCH" ]]; then
  fail "remote $REMOTE HEAD points to $remote_head_branch, expected $EXPECTED_DEFAULT_BRANCH"
fi

local_origin_head=""
local_err_file="$(mktemp)"
if local_origin_head=$(git symbolic-ref --quiet --short "refs/remotes/${REMOTE}/HEAD" 2>"$local_err_file"); then
  :
else
  local_err="$(<"$local_err_file")"
  rm -f "$local_err_file"
  fail "unable to read local refs/remotes/${REMOTE}/HEAD: ${local_err:-missing or not a symbolic ref}"
  local_origin_head=""
fi
rm -f "$local_err_file"

if [[ -n "$local_origin_head" ]]; then
  info "local refs/remotes/${REMOTE}/HEAD: $local_origin_head"
  expected_local_head="${REMOTE}/${github_default_branch:-$EXPECTED_DEFAULT_BRANCH}"
  if [[ "$local_origin_head" != "$expected_local_head" ]]; then
    fail "local refs/remotes/${REMOTE}/HEAD points to $local_origin_head, expected $expected_local_head"
  fi
fi

protection_branch="$EXPECTED_DEFAULT_BRANCH"
classic_protected=0
classic_err_file="$(mktemp)"
if [[ -n "$owner_repo" && -n "$protection_branch" ]]; then
  if gh api -H 'Accept: application/vnd.github+json' "repos/${owner_repo}/branches/${protection_branch}/protection" >/dev/null 2>"$classic_err_file"; then
    classic_protected=1
    info "classic branch protection: present"
  else
    classic_err="$(<"$classic_err_file")"
    if [[ "$classic_err" == *"HTTP 404"* ]]; then
      info "classic branch protection: absent (404)"
    else
      fail "unable to read classic branch protection for ${protection_branch}: ${classic_err:-unknown error}"
    fi
  fi
else
  fail "skipping branch-protection check because owner/repo or protected branch could not be determined"
fi
rm -f "$classic_err_file"

matching_rulesets=()
rulesets_err_file="$(mktemp)"
if [[ -n "$owner_repo" && -n "$protection_branch" ]]; then
  rulesets_json=""
  if rulesets_json=$(gh api -H 'Accept: application/vnd.github+json' "repos/${owner_repo}/rulesets" 2>"$rulesets_err_file"); then
    active_rulesets_tsv="$(printf '%s' "$rulesets_json" | python3 -c '
import json
import sys
for ruleset in json.load(sys.stdin):
    if ruleset.get("target") == "branch" and ruleset.get("enforcement") == "active":
        ruleset_id = ruleset.get("id")
        if ruleset_id is not None:
            print(f"{ruleset_id}\t{ruleset.get('"'"'name'"'"') or '"'"'(unnamed ruleset)'"'"'}")
')"

    while IFS=$'\t' read -r ruleset_id ruleset_name; do
      [[ -z "${ruleset_id:-}" ]] && continue
      detail_err_file="$(mktemp)"
      detail_json=""
      if detail_json=$(gh api -H 'Accept: application/vnd.github+json' "repos/${owner_repo}/rulesets/${ruleset_id}" 2>"$detail_err_file"); then
        if printf '%s' "$detail_json" | ruleset_applies_to_branch "${github_default_branch:-$EXPECTED_DEFAULT_BRANCH}" "$protection_branch"; then
          matching_rulesets+=("$ruleset_name")
        fi
      else
        detail_err="$(<"$detail_err_file")"
        fail "unable to read ruleset ${ruleset_id} (${ruleset_name}): ${detail_err:-unknown error}"
      fi
      rm -f "$detail_err_file"
    done <<<"$active_rulesets_tsv"
  else
    rulesets_err="$(<"$rulesets_err_file")"
    fail "unable to read repository rulesets: ${rulesets_err:-unknown error}"
  fi
fi
rm -f "$rulesets_err_file"

if [[ "${#matching_rulesets[@]}" -gt 0 ]]; then
  for ruleset_name in "${matching_rulesets[@]}"; do
    info "branch is protected by active ruleset: $ruleset_name"
  done
else
  info "active matching branch rulesets: none"
fi

if [[ "$classic_protected" -eq 0 && "${#matching_rulesets[@]}" -eq 0 ]]; then
  fail "${protection_branch} has no classic branch protection and no active matching branch ruleset"
fi

if [[ "$failures" -ne 0 ]]; then
  info "Default-branch preflight failed with $failures issue(s)."
  exit 1
fi

info "Default-branch preflight passed."
