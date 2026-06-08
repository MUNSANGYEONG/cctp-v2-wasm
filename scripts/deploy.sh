#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# 최초 배포: forwarder 코드 store → factory 코드 store → factory instantiate.
# 모든 Tx 해시 / code_id / contract address 를 deployments/<chain>.json 에 기록.
#
# 사용법: scripts/deploy.sh
# 사전: scripts/config.env 작성, scripts/build.sh 로 artifacts 준비
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

init_record

SENDER_KEY_ADDR="$(key_address)"
OWNER_ADDR="${OWNER:-${SENDER_KEY_ADDR}}"
ADMIN_ADDR="${ADMIN:-${SENDER_KEY_ADDR}}"

: "${SKIP_RELAYER_ADDR:?config.env 에 SKIP_RELAYER_ADDR 설정 필요}"
: "${SKIP_ENTRYPOINT_ADDR:?config.env 에 SKIP_ENTRYPOINT_ADDR 설정 필요}"

# ── 1) forwarder 코드 업로드 ────────────────────────────────────────────────
log "[1/3] forwarder 코드 store"
store_code "${FORWARDER_WASM}"
FWD_CODE_ID="${CODE_ID}"
record_update '
  .forwarder.code_id = ($cid|tonumber)
  | .forwarder.code_history += [{code_id:($cid|tonumber), store_tx:$tx, height:$h, created_at:(now|todate)}]
' --arg cid "${FWD_CODE_ID}" --arg tx "${STORE_TX_HASH}" --arg h "${STORE_HEIGHT}"

# ── 2) factory 코드 업로드 ──────────────────────────────────────────────────
log "[2/3] factory 코드 store"
store_code "${FACTORY_WASM}"
FAC_CODE_ID="${CODE_ID}"
FAC_STORE_TX="${STORE_TX_HASH}"

# ── 3) factory instantiate ──────────────────────────────────────────────────
log "[3/3] factory instantiate (forwarder_code_id=${FWD_CODE_ID})"
INIT_MSG="$(jq -n \
  --argjson cid "${FWD_CODE_ID}" \
  --arg r "${SKIP_RELAYER_ADDR}" \
  --arg e "${SKIP_ENTRYPOINT_ADDR}" \
  --arg o "${OWNER_ADDR}" '
  { forwarder_code_id: $cid, skip_relayer_addr: $r, skip_entrypoint_addr: $e, owner: $o }')"

tmp="$(mktemp)"
  broadcast tx wasm instantiate "${FAC_CODE_ID}" "${INIT_MSG}" \
    --label "cctp-v2-forwarder-factory" --admin "${ADMIN_ADDR}" >"${tmp}"
  TXJSON="$(cat "${tmp}")"
  rm -f "${tmp}"

FACTORY_ADDR="$(tx_attr "${TXJSON}" instantiate _contract_address)"
INST_TX="${TX_HASH}"; INST_HEIGHT="${TX_HEIGHT}"
[ -n "${FACTORY_ADDR}" ] || die "factory 주소 파싱 실패"

record_update '
  .factory = {
    code_id: ($cid|tonumber),
    address: $addr,
    admin: $admin,
    owner: $owner,
    skip_relayer_addr: $r,
    skip_entrypoint_addr: $e,
    store_tx: $stx,
    instantiate_tx: $itx,
    instantiate_height: ($h|tonumber),
    version_history: [{code_id:($cid|tonumber), action:"instantiate", tx:$itx, height:($h|tonumber), at:(now|todate)}]
  }' \
  --arg cid "${FAC_CODE_ID}" --arg addr "${FACTORY_ADDR}" --arg admin "${ADMIN_ADDR}" \
  --arg owner "${OWNER_ADDR}" --arg r "${SKIP_RELAYER_ADDR}" --arg e "${SKIP_ENTRYPOINT_ADDR}" \
  --arg stx "${FAC_STORE_TX}" --arg itx "${INST_TX}" --arg h "${INST_HEIGHT}"

echo ""
echo "=== 배포 완료 ==="
echo "  forwarder code_id : ${FWD_CODE_ID}"
echo "  factory   code_id : ${FAC_CODE_ID}"
echo "  factory   address : ${FACTORY_ADDR}"
echo "  factory   admin   : ${ADMIN_ADDR}"
echo "  기록 파일         : ${RECORD_FILE}"
echo ""
echo "다음 단계: scripts/create_forwarder.sh <sender_addr> <dest_chain> <recipient_addr>"
