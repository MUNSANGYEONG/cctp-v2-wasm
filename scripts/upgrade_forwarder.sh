#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Forwarder 코드 업그레이드.
#
#   1) 새 forwarder wasm 을 store → new_code_id
#   2) factory.UpdateConfig{forwarder_code_id:new_code_id}  (owner 키만 가능)
#
# 효과: 이후 생성되는 forwarder 는 새 코드로 instantiate2 됩니다.
#
# ⚠️ 기존 forwarder 들은 admin 이 factory 컨트랙트이며, factory 에 별도의
#    MigrateForwarder 실행 메시지가 없으므로 본 스크립트로 마이그레이션되지
#    않습니다. (= 새 코드는 신규 라우트에만 적용). 기존 forwarder 일괄 마이그레이션이
#    필요하면 factory 에 MigrateForwarder 엔드포인트 추가가 선행되어야 합니다.
#
# 사용법: scripts/upgrade_forwarder.sh
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

FACTORY_ADDR="$(record_get '.factory.address')"
[ -n "${FACTORY_ADDR}" ] || die "factory 주소가 기록에 없습니다. 먼저 deploy.sh 실행."

OLD_CODE_ID="$(record_get '.forwarder.code_id')"
log "factory=${FACTORY_ADDR} 현재 forwarder code_id=${OLD_CODE_ID}"

# ── 1) 새 forwarder 코드 store ──────────────────────────────────────────────
log "[1/2] 새 forwarder 코드 store"
store_code "${FORWARDER_WASM}"
NEW_CODE_ID="${CODE_ID}"

# ── 2) factory UpdateConfig 로 forwarder_code_id 갱신 ───────────────────────
log "[2/2] factory.UpdateConfig forwarder_code_id=${NEW_CODE_ID}"
EXEC_MSG="$(jq -n --argjson cid "${NEW_CODE_ID}" '{update_config:{forwarder_code_id:$cid}}')"
tmp="$(mktemp)"
broadcast tx wasm execute "${FACTORY_ADDR}" "${EXEC_MSG}" >"${tmp}"
TXJSON="$(cat "${tmp}")"
rm -f "${tmp}"
HEIGHT="${TX_HEIGHT}"   # TX_HASH/TX_HEIGHT 는 broadcast 가 설정

record_update '
  .forwarder.code_id = ($cid|tonumber)
  | .forwarder.code_history += [{code_id:($cid|tonumber), action:"upgrade", store_tx:$stx, update_config_tx:$tx, height:($h|tonumber), created_at:(now|todate)}]
' --arg cid "${NEW_CODE_ID}" --arg stx "${STORE_TX_HASH}" --arg tx "${TX_HASH}" --arg h "${HEIGHT}"

echo ""
echo "=== forwarder 코드 업그레이드 완료 ==="
echo "  forwarder code_id : ${OLD_CODE_ID} → ${NEW_CODE_ID}"
echo "  update_config tx  : ${TX_HASH} (height ${HEIGHT})"
echo "  기록              : ${RECORD_FILE}"
echo ""
warn "기존 forwarder 들은 이전 code_id 를 유지합니다(신규 라우트만 새 코드 적용)."
