#!/usr/bin/env bash
# Cross-document lint: fails on stale terms or broken relative Markdown links.
# Add `<!-- lint:allow -->` to a line that legitimately mentions a guarded term.
cd "$(dirname "$0")" || exit 2
fail=0

check() {
  local label="$1" pattern="$2"
  local hits
  hits=$(grep -rnE --exclude-dir=historical "$pattern" specs plans | grep -v 'lint:allow')
  if [ -n "$hits" ]; then
    echo "STALE [$label]:"
    echo "$hits" | cut -c1-200
    fail=1
  fi
}

check "AppState"                    'AppState'
check "payment_method"              'payment_method|payment method'
check "raw file copy"               'copy the file|wal_checkpoint\(TRUNCATE\)\)? and copy'
check "old export names"            'athena-transactions-|athena-profile-'
check "top_category w/o _id"        '(^|[^_a-z])top_category([^_a-z]|$)'
check "old largest title column"    'largest_expense_name'
check "ui i18n path"                'ui/i18n|ui::i18n'
check "Local::now in exporter"      'export[_a-z]*\([^)]*Local::now'
check "previous_net"                'previous_net'
check "is_complete"                 '\bis_complete\b'
check "CategoryTotal.name"          'CategoryTotal\.name'
check "schema_version kept at 1"    'Additive changes keep this at 1'
check "Option<Db>"                  'Option<Db>'
check "template person_id"          'template.*person_id|person_id.*template'
check "wallpaper egui::Image"       'wallpaper.*egui::Image|egui::Image.*wallpaper'

# deleted_at IS NULL is allowed only in the view definition.
hits=$(grep -rnE --exclude-dir=historical 'deleted_at IS NULL' specs plans | grep -v 'CREATE VIEW live_transactions' | grep -v 'lint:allow')
if [ -n "$hits" ]; then
  echo "STALE [per-query deleted filter]:"
  echo "$hits" | cut -c1-200
  fail=1
fi

# Verify every relative Markdown link under docs/ resolves to an existing file.
while IFS= read -r md; do
  # Strip fenced code blocks and inline code spans before scanning links.
  while IFS= read -r link; do
    target=${link#*](}
    target=${target%)}
    target=${target%% *}
    target=${target%%#*}
    case "$target" in
      ""|http://*|https://*|mailto:*|file://*|*\<*|*\>*) continue ;;
    esac
    if ! (cd "$(dirname "$md")" && [ -e "$target" ]); then
      echo "BROKEN LINK [$md]: $target"
      fail=1
    fi
  done < <(
    awk '
      /^[[:space:]]*````/ { fence4 = !fence4; next }
      !fence4 && /^[[:space:]]*```/ { fence3 = !fence3; next }
      !fence3 && !fence4 { print }
    ' "$md" | sed -E 's/`[^`]*`//g' | grep -oE '\[[^]]+\]\([^)]+\)' || true
  )
done < <(find .. -name '*.md' | sort)

[ $fail -eq 0 ] && echo "doc-lint: clean"
exit $fail
