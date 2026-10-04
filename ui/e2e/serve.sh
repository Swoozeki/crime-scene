#!/usr/bin/env bash
# Builds a small scripted git repo, then serves it with csi for the Playwright smoke tests.
set -euo pipefail
CSI="${CSI_BIN:-$(cd "$(dirname "$0")/../.." && pwd)/target/release/csi}"
PORT="${PORT:-7788}"
DIR="$(mktemp -d)/shop"
mkdir -p "$DIR" && cd "$DIR"
git init -q -b main
git config user.email fixture@example.com && git config user.name Fixture
commit() { # author days_ago message
  local d=$(( $(date +%s) - $2 * 86400 ))
  git add -A
  GIT_AUTHOR_NAME="$1" GIT_AUTHOR_EMAIL="$(echo "$1" | tr 'A-Z' 'a-z')@shop.io" GIT_AUTHOR_DATE="@$d +0000" \
  GIT_COMMITTER_NAME="$1" GIT_COMMITTER_EMAIL="ci@shop.io" GIT_COMMITTER_DATE="@$d +0000" \
    git commit -q -m "$3"
}
mkdir -p src/app/cart src/app/pricing src/app/legacy
for i in $(seq 1 30); do
  { echo "import { Component } from '@angular/core';"
    echo "@Component({ selector: 'app-cart', templateUrl: './cart.component.html' })"
    echo "export class CartComponent {"
    echo "  total(x: number) { let r = 0;"
    for j in $(seq 0 "$i"); do echo "    if (x > $j) { r += $j; }"; done
    echo "    return r; }"
    echo "}"; } > src/app/cart/cart.component.ts
  echo "<div>@if (a) { <p>$i</p> }</div>" > src/app/cart/cart.component.html
  if (( i % 4 != 0 )); then echo "export function price(x: number) { return x * $i; }" > src/app/pricing/pricing.service.ts; fi
  if (( i < 6 )); then echo "export function legacy() { return $i; }" > src/app/legacy/legacy.ts; fi
  author=Alice; (( i % 3 == 0 )) && author=Bob; (( i < 6 )) && author=Carl
  msg="feat: cart $i"; (( i % 5 == 0 )) && msg="fix: cart $i"
  commit "$author" $(( 400 - i * 12 )) "$msg"
done
"$CSI" scan -q
exec "$CSI" serve --port "$PORT"
