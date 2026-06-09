#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# forwarder 컨트랙트 config 조회 스크립트.
#
# 사용법: scripts/query_forwarder.sh <forwarder_address>
#   forwarder_address : 조회할 forwarder 컨트랙트 주소
#
# 사전: scripts/config.env 작성 (BINARY, CHAIN_ID, NODE 등)
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

ADDR="${1:-}"
[ -n "${ADDR}" ] || die "forwarder 주소가 필요합니다.\n  사용법: $0 <forwarder_address>"

log "forwarder config 조회: ${ADDR}"
query_smart "${ADDR}" '{"config":{}}' | jq .
