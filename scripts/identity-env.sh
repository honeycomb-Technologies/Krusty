#!/usr/bin/env bash

# Previous-generation environment compatibility is intentionally isolated in
# this file. Canonical variables always win, including when deliberately set to
# an empty value.
mitsuro_import_legacy_identity_env() {
  local legacy_name canonical_name suffix

  while IFS= read -r legacy_name; do
    case "$legacy_name" in
      KRUSTY_MAKO_*)
        suffix=${legacy_name#KRUSTY_MAKO_}
        canonical_name="MITSURO_HIVE_$suffix"
        ;;
      KRUSTY_*)
        suffix=${legacy_name#KRUSTY_}
        canonical_name="MITSURO_$suffix"
        ;;
      *)
        continue
        ;;
    esac

    if [[ ! -v "$canonical_name" ]]; then
      printf -v "$canonical_name" '%s' "${!legacy_name}"
      export "$canonical_name"
    fi
  done < <(compgen -A variable 'KRUSTY_')
}

identity_env_self_test() {
  (
    unset MITSURO_TEST_VALUE
    KRUSTY_TEST_VALUE=previous
    mitsuro_import_legacy_identity_env
    [[ "$MITSURO_TEST_VALUE" == previous ]]
  )
  (
    MITSURO_TEST_VALUE=
    KRUSTY_TEST_VALUE=previous
    mitsuro_import_legacy_identity_env
    [[ -v MITSURO_TEST_VALUE && -z "$MITSURO_TEST_VALUE" ]]
  )
  (
    MITSURO_TEST_VALUE=canonical
    KRUSTY_TEST_VALUE=previous
    mitsuro_import_legacy_identity_env
    [[ "$MITSURO_TEST_VALUE" == canonical ]]
  )
  (
    unset MITSURO_HIVE_TEST_SOCKET
    KRUSTY_MAKO_TEST_SOCKET=previous-hive
    mitsuro_import_legacy_identity_env
    [[ "$MITSURO_HIVE_TEST_SOCKET" == previous-hive ]]
  )
  printf 'identity-env self-test passed\n'
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  set -euo pipefail
  if [[ "${1:-}" != --self-test || $# -ne 1 ]]; then
    printf 'usage: %s --self-test\n' "$0" >&2
    exit 2
  fi
  identity_env_self_test
else
  mitsuro_import_legacy_identity_env
  unset -f mitsuro_import_legacy_identity_env identity_env_self_test
fi
