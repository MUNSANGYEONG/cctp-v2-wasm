#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# (sender, dest_chain, recipient) 라우트에 대한 forwarder 를 factory 를 통해
# instantiate2 로 생성합니다. 생성 전 예측 주소를 먼저 조회·출력하므로
# (사전 펀딩 가능), 생성 후 예측 == 실제 주소 일치를 검증합니다.
#
# 사용법:
#   scripts/create_forwarder.sh <sender_addr> <dest_chain> <recipient_addr> [factory_addr]
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

SENDER_ADDR="${1:?사용법: create_forwarder.sh <sender_addr> <dest_chain> <recipient_addr> [factory_addr]}"
DEST_CHAIN="${2:?dest_chain 누락}"
RECIPIENT_ADDR="${3:?recipient_addr 누락}"
FACTORY_ADDR="${4:-$(record_get '.factory.address')}"
[ -n "${FACTORY_ADDR}" ] || die "factory 주소를 찾을 수 없습니다. 인자로 넘기거나 먼저 deploy.sh 실행."

# ── 예측 주소 조회 (생성 전) ─────────────────────────────────────────────────
QADDR="$(jq -n --arg s "${SENDER_ADDR}" --arg d "${DEST_CHAIN}" --arg r "${RECIPIENT_ADDR}" \
  '{forwarder_address:{sender_addr:$s, dest_chain:$d, recipient_addr:$r}}')"
# 명령치환으로 받아야 query_smart 의 die 가 set -e 로 전파됨(프로세스치환은 X).
PRED_DATA="$(query_smart "${FACTORY_ADDR}" "${QADDR}")"
read -r PREDICTED ALREADY < <(jq -r '"\(.address) \(.exists)"' <<<"${PRED_DATA}")
[ -n "${PREDICTED}" ] && [ "${PREDICTED}" != "null" ] || die "예측 주소 파싱 실패: ${PRED_DATA}"
log "예측 forwarder 주소: ${PREDICTED} (exists=${ALREADY})"
[ "${ALREADY}" = "true" ] && die "이미 존재하는 라우트입니다: ${PREDICTED}"

# ── CreateForwarder 실행 ────────────────────────────────────────────────────
EXEC_MSG="$(jq -n --arg s "${SENDER_ADDR}" --arg d "${DEST_CHAIN}" --arg r "${RECIPIENT_ADDR}" \
  '{create_forwarder:{sender_addr:$s, dest_chain:$d, recipient_addr:$r}}')"
tmp="$(mktemp)"
broadcast tx wasm execute "${FACTORY_ADDR}" "${EXEC_MSG}" >"${tmp}"
TXJSON="$(cat "${tmp}")"
rm -f "${tmp}"

ACTUAL="$(tx_attr "${TXJSON}" instantiate _contract_address)"
[ -n "${ACTUAL}" ] || ACTUAL="$(tx_attr "${TXJSON}" wasm forwarder)"
SALT="$(tx_attr "${TXJSON}" wasm salt)"
HEIGHT="${TX_HEIGHT}"   # TX_HASH/TX_HEIGHT 는 broadcast 가 설정
FWD_CODE_ID="$(record_get '.forwarder.code_id')"

# instantiate2 의 핵심 안전보장: 예측==실제. 불일치는 체인/코드 불일치이므로 중단.
[ -n "${ACTUAL}" ] || die "생성된 forwarder 주소 파싱 실패: ${TXJSON}"
[ "${ACTUAL}" = "${PREDICTED}" ] || die "예측(${PREDICTED}) != 실제(${ACTUAL}) — instantiate2 불일치, 사전펀딩 주소 위험"

record_update '
  .forwarders += [{
    sender_addr: $s, dest_chain: $d, recipient_addr: $r,
    address: $addr, salt: $salt, code_id: ($cid|tonumber),
    create_tx: $tx, height: ($h|tonumber), created_at: (now|todate)
  }]' \
  --arg s "${SENDER_ADDR}" --arg d "${DEST_CHAIN}" --arg r "${RECIPIENT_ADDR}" \
  --arg addr "${ACTUAL}" --arg salt "${SALT}" --arg cid "${FWD_CODE_ID:-0}" \
  --arg tx "${TX_HASH}" --arg h "${HEIGHT}"

echo ""
echo "=== forwarder 생성 완료 ==="
echo "  주소     : ${ACTUAL}"
echo "  salt     : ${SALT}"
echo "  code_id  : ${FWD_CODE_ID}"
echo "  tx       : ${TX_HASH} (height ${HEIGHT})"
echo "  기록     : ${RECORD_FILE}"
