#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# forwarder 컨트랙트 Refund 실행 스크립트.
# 컨트랙트에 남은 잔액을 sender_addr(환불 주소)로 반환한다.
#
# 사용법: scripts/refund.sh <forwarder_address>
#   forwarder_address : 환불을 실행할 forwarder 컨트랙트 주소
#
# 사전: scripts/config.env 작성 (BINARY, KEY, CHAIN_ID, NODE, GAS_PRICES 등)
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

ADDR="${1:-}"
[ -n "${ADDR}" ] || die "forwarder 주소가 필요합니다.\n  사용법: $0 <forwarder_address>"

# ── 1) config 조회 ────────────────────────────────────────────────────────────
log "forwarder config 조회: ${ADDR}"
CONFIG="$(query_smart "${ADDR}" '{"config":{}}')"
echo "${CONFIG}" | jq .

# ── 2) 잔고 확인 ──────────────────────────────────────────────────────────────
log "컨트랙트 잔고 확인"
"${BINARY}" query bank balances "${ADDR}" \
  --node "${NODE}" --output json | jq '.balances'

# ── 3) Refund 실행 ────────────────────────────────────────────────────────────
log "Refund 실행 → ${ADDR}"
tmp="$(mktemp)"
broadcast tx wasm execute "${ADDR}" '{"refund":{}}' >"${tmp}"
rm -f "${tmp}"

log "tx 완료: txhash=${TX_HASH} height=${TX_HEIGHT}"
