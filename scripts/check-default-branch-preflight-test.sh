#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SCRIPT_DIR/check-default-branch-preflight.sh"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  if [[ "$haystack" != *"$needle"* ]]; then
    fail "expected output to contain: $needle
--- output ---
$haystack
--- end output ---"
  fi
}

with_fake_commands() {
  local fakebin="$1"
  mkdir -p "$fakebin"

  cat >"$fakebin/gh" <<'GH'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "repo" && "${2:-}" == "view" ]]; then
  printf '{"nameWithOwner":"honeycomb-Technologies/Krusty","defaultBranchRef":{"name":"%s"}}\n' "${GITHUB_DEFAULT_BRANCH:-main}"
  exit 0
fi

if [[ "${1:-}" == "api" ]]; then
  shift
  endpoint=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      -H|--header|--jq)
        shift 2
        ;;
      *)
        endpoint="$1"
        shift
        ;;
    esac
  done

  case "$endpoint" in
    repos/honeycomb-Technologies/Krusty/branches/*/protection)
      if [[ "${CLASSIC_PROTECTED:-0}" == "1" ]]; then
        printf '{"url":"https://api.github.com/repos/honeycomb-Technologies/Krusty/branches/main/protection"}\n'
      else
        echo 'gh: Branch not protected (HTTP 404)' >&2
        exit 1
      fi
      ;;
    repos/honeycomb-Technologies/Krusty/rulesets)
      if [[ "${RULESET_PROTECTED:-0}" == "1" ]]; then
        printf '[{"id":12308175,"name":"Protect main","target":"branch","enforcement":"active"}]\n'
      else
        printf '[]\n'
      fi
      ;;
    repos/honeycomb-Technologies/Krusty/rulesets/12308175)
      printf '{"id":12308175,"name":"Protect main","target":"branch","enforcement":"active","conditions":{"ref_name":{"include":["refs/heads/main"],"exclude":[]}},"rules":[{"type":"deletion"},{"type":"non_fast_forward"},{"type":"required_status_checks"}]}\n'
      ;;
    *)
      echo "unexpected gh api endpoint: $endpoint" >&2
      exit 64
      ;;
  esac
  exit 0
fi

echo "unexpected gh invocation: $*" >&2
exit 64
GH
  chmod +x "$fakebin/gh"

  cat >"$fakebin/git" <<'GIT'
#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
  ls-remote)
    if [[ "${2:-}" == "--symref" && "${3:-}" == "origin" && "${4:-}" == "HEAD" ]]; then
      printf 'ref: refs/heads/%s\tHEAD\n' "${REMOTE_HEAD_BRANCH:-main}"
      printf '0000000000000000000000000000000000000000\tHEAD\n'
      exit 0
    fi
    ;;
  symbolic-ref)
    if [[ "${2:-}" == "--quiet" && "${3:-}" == "--short" && "${4:-}" == "refs/remotes/origin/HEAD" ]]; then
      if [[ "${LOCAL_ORIGIN_HEAD_BRANCH:-main}" == "__missing" ]]; then
        exit 1
      fi
      printf 'origin/%s\n' "${LOCAL_ORIGIN_HEAD_BRANCH:-main}"
      exit 0
    fi
    ;;
esac

echo "unexpected git invocation: $*" >&2
exit 64
GIT
  chmod +x "$fakebin/git"
}

run_preflight() {
  local expected_rc="$1"
  shift
  local tmpdir fakebin output rc
  tmpdir="$(mktemp -d)"
  fakebin="$tmpdir/bin"
  with_fake_commands "$fakebin"

  set +e
  output=$(PATH="$fakebin:$PATH" "$@" "$SCRIPT" 2>&1)
  rc=$?
  set -e
  rm -rf "$tmpdir"

  if [[ "$expected_rc" == "0" && "$rc" -ne 0 ]]; then
    fail "expected success but got rc=$rc
--- output ---
$output
--- end output ---"
  fi
  if [[ "$expected_rc" != "0" && "$rc" -eq 0 ]]; then
    fail "expected failure but got success
--- output ---
$output
--- end output ---"
  fi

  printf '%s' "$output"
}

test_fails_when_local_origin_head_is_stale() {
  local output
  output=$(run_preflight 1 env GITHUB_DEFAULT_BRANCH=main REMOTE_HEAD_BRANCH=main LOCAL_ORIGIN_HEAD_BRANCH=dev RULESET_PROTECTED=1 CLASSIC_PROTECTED=0)
  assert_contains "$output" "GitHub default branch: main"
  assert_contains "$output" "remote origin HEAD: main"
  assert_contains "$output" "local refs/remotes/origin/HEAD: origin/dev"
  assert_contains "$output" "branch is protected by active ruleset: Protect main"
  assert_contains "$output" "FAIL: local refs/remotes/origin/HEAD points to origin/dev, expected origin/main"
}

test_passes_when_heads_align_and_ruleset_protects_main() {
  local output
  output=$(run_preflight 0 env GITHUB_DEFAULT_BRANCH=main REMOTE_HEAD_BRANCH=main LOCAL_ORIGIN_HEAD_BRANCH=main RULESET_PROTECTED=1 CLASSIC_PROTECTED=0)
  assert_contains "$output" "GitHub default branch: main"
  assert_contains "$output" "remote origin HEAD: main"
  assert_contains "$output" "local refs/remotes/origin/HEAD: origin/main"
  assert_contains "$output" "branch is protected by active ruleset: Protect main"
  assert_contains "$output" "Default-branch preflight passed."
}

test_fails_when_remote_head_disagrees_with_github_default() {
  local output
  output=$(run_preflight 1 env GITHUB_DEFAULT_BRANCH=main REMOTE_HEAD_BRANCH=dev LOCAL_ORIGIN_HEAD_BRANCH=main RULESET_PROTECTED=1 CLASSIC_PROTECTED=0)
  assert_contains "$output" "GitHub default branch: main"
  assert_contains "$output" "remote origin HEAD: dev"
  assert_contains "$output" "FAIL: remote origin HEAD points to dev, expected main"
}

test_fails_when_main_has_no_classic_or_ruleset_protection() {
  local output
  output=$(run_preflight 1 env GITHUB_DEFAULT_BRANCH=main REMOTE_HEAD_BRANCH=main LOCAL_ORIGIN_HEAD_BRANCH=main RULESET_PROTECTED=0 CLASSIC_PROTECTED=0)
  assert_contains "$output" "classic branch protection: absent (404)"
  assert_contains "$output" "FAIL: main has no classic branch protection and no active matching branch ruleset"
}

test_fails_when_github_default_is_not_main() {
  local output
  output=$(run_preflight 1 env GITHUB_DEFAULT_BRANCH=dev REMOTE_HEAD_BRANCH=main LOCAL_ORIGIN_HEAD_BRANCH=main RULESET_PROTECTED=1 CLASSIC_PROTECTED=0)
  assert_contains "$output" "GitHub default branch: dev"
  assert_contains "$output" "FAIL: GitHub default branch is dev, expected main"
}

main() {
  test_fails_when_local_origin_head_is_stale
  test_passes_when_heads_align_and_ruleset_protects_main
  test_fails_when_remote_head_disagrees_with_github_default
  test_fails_when_main_has_no_classic_or_ruleset_protection
  test_fails_when_github_default_is_not_main
  echo "check-default-branch-preflight tests passed"
}

main "$@"
