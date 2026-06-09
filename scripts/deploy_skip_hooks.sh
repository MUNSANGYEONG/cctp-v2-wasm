#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# skip-go IBC Hooks 버전 배포:
#   1) ibc_adapter (ibc_hooks) 코드 store → instantiate (임시 entry_point 주소)
#   2) entry_point 코드 store → instantiate (adapter 주소 참조)
#   3) adapter migrate → 실제 entry_point 주소로 갱신
#
# 사용법: scripts/deploy_skip_hooks.sh
# 사전: scripts/config.env 작성, artifacts/ 에 wasm 파일 준비
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

SENDER_KEY_ADDR="$(key_address)"
ADMIN_ADDR="${ADMIN:-${SENDER_KEY_ADDR}}"

# wasm 경로
HOOKS_ADAPTER_WASM="${HOOKS_ADAPTER_WASM:-${ROOT_DIR}/artifacts/skip_go_ibc_adapter_ibc_hooks.wasm}"
ENTRY_POINT_WASM="${ENTRY_POINT_WASM:-${ROOT_DIR}/artifacts/skip_go_entry_point.wasm}"

# deployments/<chain>_skip_hooks.json 에 별도 기록
RECORD_FILE="${DEPLOY_DIR}/${CHAIN_ID}_skip_hooks.json"
mkdir -p "${DEPLOY_DIR}"
if [ ! -f "${RECORD_FILE}" ]; then
  jq -n --arg cid "${CHAIN_ID}" --arg bin "${BINARY}" '{
    chain_id: $cid, binary: $bin, variant: "ibc_hooks",
    updated_at: (now|todate),
    ibc_adapter: {}, entry_point: {}
  }' >"${RECORD_FILE}"
fi

# record_update 는 lib.sh 의 RECORD_FILE 을 참조하므로 같은 변수 사용
# (lib.sh 는 이미 RECORD_FILE 을 export 하지 않으므로 직접 in-place 처리)
skip_record_update() {
  local filter="$1"; shift
  local tmp; tmp="$(mktemp)"
  jq "$@" "(${filter}) | .updated_at = (now|todate)" "${RECORD_FILE}" >"${tmp}" \
    && mv "${tmp}" "${RECORD_FILE}"
}

# ── 1) ibc_hooks adapter 코드 store ─────────────────────────────────────────
log "[1/5] ibc_hooks adapter 코드 store"
store_code "${HOOKS_ADAPTER_WASM}"
ADAPTER_CODE_ID="${CODE_ID}"
skip_record_update '
  .ibc_adapter.code_id = ($cid|tonumber)
  | .ibc_adapter.code_history += [{code_id:($cid|tonumber), store_tx:$tx, height:($h|tonumber), created_at:(now|todate)}]
' --arg cid "${ADAPTER_CODE_ID}" --arg tx "${STORE_TX_HASH}" --arg h "${STORE_HEIGHT}"

# ── 2) ibc_hooks adapter instantiate (임시 주소 사용) ───────────────────────
# entry_point 주소를 미리 알 수 없으므로 배포자 주소를 임시로 사용.
# 이후 migrate 로 실제 entry_point 주소로 교체한다.
log "[2/5] ibc_hooks adapter instantiate (임시 entry_point=${SENDER_KEY_ADDR})"
ADAPTER_INIT_MSG="$(jq -n --arg ep "${SENDER_KEY_ADDR}" \
  '{"entry_point_contract_address": $ep}')"

ADAPTER_INST_JSON="$(broadcast tx wasm instantiate "${ADAPTER_CODE_ID}" "${ADAPTER_INIT_MSG}" \
  --label "skip-go-ibc-adapter-ibc-hooks" --admin "${ADMIN_ADDR}")"
ADAPTER_ADDR="$(tx_attr "${ADAPTER_INST_JSON}" instantiate _contract_address)"
[ -n "${ADAPTER_ADDR}" ] || die "adapter 주소 파싱 실패"
log "→ adapter=${ADAPTER_ADDR}"

skip_record_update '
  .ibc_adapter.address       = $addr
  | .ibc_adapter.admin       = $admin
  | .ibc_adapter.instantiate_tx = $tx
  | .ibc_adapter.instantiate_height = ($h|tonumber)
' --arg addr "${ADAPTER_ADDR}" --arg admin "${ADMIN_ADDR}" \
  --arg tx "${TX_HASH}" --arg h "${TX_HEIGHT}"

# ── 3) entry_point 코드 store ────────────────────────────────────────────────
log "[3/5] entry_point 코드 store"
store_code "${ENTRY_POINT_WASM}"
EP_CODE_ID="${CODE_ID}"
skip_record_update '
  .entry_point.code_id = ($cid|tonumber)
  | .entry_point.code_history += [{code_id:($cid|tonumber), store_tx:$tx, height:($h|tonumber), created_at:(now|todate)}]
' --arg cid "${EP_CODE_ID}" --arg tx "${STORE_TX_HASH}" --arg h "${STORE_HEIGHT}"

# ── 4) entry_point instantiate ───────────────────────────────────────────────
log "[4/5] entry_point instantiate (ibc_transfer=${ADAPTER_ADDR})"
EP_INIT_MSG="$(jq -n --arg ibc "${ADAPTER_ADDR}" '{
  "swap_venues": [],
  "ibc_transfer_contract_address": $ibc,
  "hyperlane_transfer_contract_address": null
}')"

EP_INST_JSON="$(broadcast tx wasm instantiate "${EP_CODE_ID}" "${EP_INIT_MSG}" \
  --label "skip-go-entry-point-ibc-hooks" --admin "${ADMIN_ADDR}")"
EP_ADDR="$(tx_attr "${EP_INST_JSON}" instantiate _contract_address)"
[ -n "${EP_ADDR}" ] || die "entry_point 주소 파싱 실패"
log "→ entry_point=${EP_ADDR}"

skip_record_update '
  .entry_point.address            = $addr
  | .entry_point.admin            = $admin
  | .entry_point.instantiate_tx   = $tx
  | .entry_point.instantiate_height = ($h|tonumber)
' --arg addr "${EP_ADDR}" --arg admin "${ADMIN_ADDR}" \
  --arg tx "${TX_HASH}" --arg h "${TX_HEIGHT}"

# ── 5) adapter migrate → 실제 entry_point 주소로 갱신 ───────────────────────
log "[5/5] adapter migrate (entry_point=${EP_ADDR})"
MIGRATE_MSG="$(jq -n --arg ep "${EP_ADDR}" \
  '{"entry_point_contract_address": $ep}')"

MIGRATE_JSON="$(broadcast tx wasm migrate "${ADAPTER_ADDR}" "${ADAPTER_CODE_ID}" "${MIGRATE_MSG}")"

skip_record_update '
  .ibc_adapter.migrate_to_ep_tx     = $tx
  | .ibc_adapter.migrate_to_ep_height = ($h|tonumber)
  | .ibc_adapter.entry_point_addr   = $ep
' --arg tx "${TX_HASH}" --arg h "${TX_HEIGHT}" --arg ep "${EP_ADDR}"

echo ""
echo "=== skip-go ibc_hooks 배포 완료 ==="
echo "  adapter    code_id : ${ADAPTER_CODE_ID}"
echo "  adapter    address : ${ADAPTER_ADDR}"
echo "  entry_point code_id: ${EP_CODE_ID}"
echo "  entry_point address: ${EP_ADDR}"
echo "  기록 파일          : ${RECORD_FILE}"
echo ""
