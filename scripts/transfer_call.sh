#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# TransferCall 실행 스크립트
# forwarder 컨트랙트에 hook_data를 전달해 Skip EntryPoint로 IBC 전송을 포워딩한다.
#
# 사용법: scripts/transfer_call.sh [forwarder_addr] [hook_data_file]
#   forwarder_addr : 포워더 컨트랙트 주소 (기본값: FORWARDER_ADDR 환경변수)
#   hook_data_file : hook_data JSON 파일 경로 (기본값: contracts/forwarder/test-data/ibc-transfer-hookdata.json)
#
# 사전: scripts/config.env 작성 (BINARY, KEY, CHAIN_ID, NODE, GAS_PRICES 등)
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

FORWARDER_ADDR="${1:-${FORWARDER_ADDR:-}}"
HOOK_DATA_FILE="${2:-${ROOT_DIR}/contracts/forwarder/test-data/ibc-transfer-hookdata.json}"

[ -n "${FORWARDER_ADDR}" ] || die "포워더 주소가 필요합니다.\n  사용법: $0 <forwarder_addr> [hook_data_file]\n  또는 FORWARDER_ADDR 환경변수 설정"
[ -f "${HOOK_DATA_FILE}" ] || die "hook_data 파일을 찾을 수 없습니다: ${HOOK_DATA_FILE}"

# ── 1) config 조회 ────────────────────────────────────────────────────────────
log "forwarder config 조회: ${FORWARDER_ADDR}"
query_smart "${FORWARDER_ADDR}" '{"config":{}}' | jq .

# ── 2) 잔고 확인 ──────────────────────────────────────────────────────────────
SEND_DENOM=$(jq -r '.. | objects | select(has("denom")) | .denom' "${HOOK_DATA_FILE}" | head -1)
log "컨트랙트 잔고 확인 (denom: ${SEND_DENOM})"
"${BINARY}" query bank balance "${FORWARDER_ADDR}" "${SEND_DENOM}" \
  --node "${NODE}" --output json | jq '{denom: .balance.denom, amount: .balance.amount}'

# ── 3) hook_data → ExecuteMsg 구성 ───────────────────────────────────────────
# timeout_timestamp: 현재 시각(ns) + 1시간 여유
TIMEOUT_NS=$(( $(date +%s) * 1000000000 + 3600000000000 ))
log "timeout_timestamp (ns): ${TIMEOUT_NS}"

HOOK_DATA=$(jq -c --argjson ts "${TIMEOUT_NS}" \
  '(.. | objects | select(has("timeout_timestamp"))).timeout_timestamp |= $ts' \
  "${HOOK_DATA_FILE}")
EXECUTE_MSG=$(jq -n --arg h "${HOOK_DATA}" '{"transfer_call":{"hook_data":$h}}')

log "hook_data 파일: ${HOOK_DATA_FILE}"
log "ExecuteMsg 구성 완료"

# ── 4) TransferCall 실행 ──────────────────────────────────────────────────────
log "TransferCall 실행 (from=${KEY} → forwarder=${FORWARDER_ADDR})"
tmp="$(mktemp)"
broadcast tx wasm execute "${FORWARDER_ADDR}" "${EXECUTE_MSG}" >"${tmp}"
rm -f "${tmp}"

log "TX hash : ${TX_HASH}"
log "height  : ${TX_HEIGHT}"

echo ""
echo "=== TransferCall 완료 ==="
echo "  forwarder : ${FORWARDER_ADDR}"
echo "  hook_data : ${HOOK_DATA_FILE}"
echo "  tx_hash   : ${TX_HASH}"
echo "  height    : ${TX_HEIGHT}"
echo ""
