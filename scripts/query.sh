#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# 조회 편의 스크립트.
#
# 사용법:
#   scripts/query.sh config                                  # factory 설정
#   scripts/query.sh list [start_after] [limit]              # 등록 forwarder 목록
#   scripts/query.sh addr <sender> <dest_chain> <recipient>  # 예측 주소/존재여부
#   scripts/query.sh record                                  # 로컬 배포 기록(JSON)
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

CMD="${1:-record}"; shift || true
FACTORY_ADDR="$(record_get '.factory.address')"

# record 외 모든 명령은 factory 주소가 필요
[ "${CMD}" = record ] || [ -n "${FACTORY_ADDR}" ] || die "factory 주소 없음"

case "${CMD}" in
  config)
    query_smart "${FACTORY_ADDR}" '{"config":{}}' ;;
  list)
    Q="$(jq -n --arg sa "${1:-}" --argjson lim "${2:-20}" \
      '{forwarders: ({limit:$lim} + (if $sa=="" then {} else {start_after:$sa} end))}')"
    query_smart "${FACTORY_ADDR}" "${Q}" ;;
  addr)
    Q="$(jq -n --arg s "${1:?sender}" --arg d "${2:?dest_chain}" --arg r "${3:?recipient}" \
      '{forwarder_address:{sender_addr:$s, dest_chain:$d, recipient_addr:$r}}')"
    query_smart "${FACTORY_ADDR}" "${Q}" ;;
  record)
    print_record ;;
  *)
    die "알 수 없는 명령: ${CMD} (config|list|addr|record)" ;;
esac
