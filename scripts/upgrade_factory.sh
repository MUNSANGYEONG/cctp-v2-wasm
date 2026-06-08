#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Factory 코드 업그레이드: 새 factory wasm 을 store → 기존 factory 컨트랙트를
# 새 code_id 로 migrate. migrate 는 factory 의 admin 키(= config.env KEY)만 가능.
#
# 사용법: scripts/upgrade_factory.sh [migrate_msg_json]
#   migrate_msg 기본값: '{}' (no-op cw2 버전 갱신)
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

MIGRATE_MSG="${1:-}"; [ -n "${MIGRATE_MSG}" ] || MIGRATE_MSG='{}'
FACTORY_ADDR="$(record_get '.factory.address')"
[ -n "${FACTORY_ADDR}" ] || die "factory 주소가 기록에 없습니다. 먼저 deploy.sh 실행."

OLD_CODE_ID="$(record_get '.factory.code_id')"
log "factory=${FACTORY_ADDR} 현재 code_id=${OLD_CODE_ID}"

# ── 1) 새 factory 코드 store ────────────────────────────────────────────────
log "[1/2] 새 factory 코드 store"
store_code "${FACTORY_WASM}"
NEW_CODE_ID="${CODE_ID}"

# ── 2) migrate ──────────────────────────────────────────────────────────────
log "[2/2] migrate ${FACTORY_ADDR} → code_id ${NEW_CODE_ID}"
tmp="$(mktemp)"
broadcast tx wasm migrate "${FACTORY_ADDR}" "${NEW_CODE_ID}" "${MIGRATE_MSG}" >"${tmp}"
TXJSON="$(cat "${tmp}")"
rm -f "${tmp}"
HEIGHT="${TX_HEIGHT}"   # TX_HASH/TX_HEIGHT 는 broadcast 가 설정

record_update '
  .factory.code_id = ($cid|tonumber)
  | .factory.version_history += [{code_id:($cid|tonumber), action:"migrate", tx:$tx, height:($h|tonumber), at:(now|todate)}]
' --arg cid "${NEW_CODE_ID}" --arg tx "${TX_HASH}" --arg h "${HEIGHT}"

echo ""
echo "=== factory 업그레이드 완료 ==="
echo "  ${OLD_CODE_ID} → ${NEW_CODE_ID}"
echo "  migrate tx : ${TX_HASH} (height ${HEIGHT})"
echo "  기록       : ${RECORD_FILE}"
