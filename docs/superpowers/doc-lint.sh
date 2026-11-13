#!/usr/bin/env bash
# Cross-document lint: fails on stale terms. Add `lint:allow` to a line that legitimately mentions one.
cd "$(dirname "$0")" || exit 2
fail=0
check() {
  local label="$1" pattern="$2"
  local hits
  hits=$(grep -rnE "$pattern" specs plans | grep -v 'lint:allow')
  if [ -n "$hits" ]; then
    echo "STALE [$label]:"; echo "$hits" | cut -c1-200; fail=1
  fi
}
check "AppState"               'AppState'
check "payment_method"         'payment_method|payment method'
check "raw file copy"          'copy the file|wal_checkpoint\(TRUNCATE\)\)? and copy'
check "old export names"       'athena-transactions-|athena-profile-'
check "top_category w/o _id"   '(^|[^_a-z])top_category([^_a-z]|$)'
check "old largest title column" 'largest_expense_name'
check "ui i18n path"           'ui/i18n|ui::i18n'
check "Local::now in exporter" 'export[_a-z]*\([^)]*Local::now'
# deleted_at IS NULL is allowed only in the view definition.
hits=$(grep -rnE 'deleted_at IS NULL' specs plans | grep -v 'CREATE VIEW live_transactions' | grep -v 'lint:allow')
if [ -n "$hits" ]; then echo "STALE [per-query deleted filter]:"; echo "$hits" | cut -c1-200; fail=1; fi
[ $fail -eq 0 ] && echo "doc-lint: clean"
exit $fail
