#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# 공용 라이브러리: 설정 로드 · 트랜잭션 브로드캐스트 · 이벤트 파싱 · 배포 기록 관리.
# 다른 스크립트에서 `source "$(dirname "$0")/lib.sh"` 로 불러 사용합니다.
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

# 저장소 루트 / scripts 디렉터리
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

# ── 로그 (stdout 은 데이터 전용, 로그는 stderr 로) ───────────────────────────
log()  { printf '\033[1;34m[%s]\033[0m %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
warn() { printf '\033[1;33m[warn]\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[err]\033[0m %s\n' "$*" >&2; exit 1; }

# ── 의존 도구 확인 ───────────────────────────────────────────────────────────
command -v jq >/dev/null 2>&1 || die "jq 가 필요합니다 (brew install jq)."

# ── 설정 로드 ────────────────────────────────────────────────────────────────
CONFIG_FILE="${CONFIG_FILE:-${SCRIPT_DIR}/config.env}"
[ -f "${CONFIG_FILE}" ] || die "설정 파일이 없습니다: ${CONFIG_FILE}
  → cp scripts/config.env.example scripts/config.env 후 값을 채우세요."
# shellcheck disable=SC1090
source "${CONFIG_FILE}"

: "${BINARY:?BINARY 미설정}"
: "${CHAIN_ID:?CHAIN_ID 미설정}"
: "${NODE:?NODE 미설정}"
: "${KEY:?KEY 미설정}"
: "${KEYRING_BACKEND:=test}"
: "${GAS_ADJUSTMENT:=1.5}"
: "${GAS_PRICES:?GAS_PRICES 미설정}"

command -v "${BINARY}" >/dev/null 2>&1 || die "체인 바이너리를 찾을 수 없습니다: ${BINARY}"

# wasm 산출물 경로
FORWARDER_WASM="${FORWARDER_WASM:-${ROOT_DIR}/artifacts/forwarder.wasm}"
FACTORY_WASM="${FACTORY_WASM:-${ROOT_DIR}/artifacts/forwarder_factory.wasm}"

# 배포 기록 파일 (체인별)
DEPLOY_DIR="${ROOT_DIR}/deployments"
RECORD_FILE="${DEPLOY_DIR}/${CHAIN_ID}.json"

# 공통 트랜잭션 플래그
TX_FLAGS=(
  --from "${KEY}"
  --chain-id "${CHAIN_ID}"
  --node "${NODE}"
  --keyring-backend "${KEYRING_BACKEND}"
  --gas auto
  --gas-adjustment "${GAS_ADJUSTMENT}"
  --gas-prices "${GAS_PRICES}"
  --broadcast-mode sync
  --output json
  -y
)

# ── 주소 조회 ────────────────────────────────────────────────────────────────
key_address() {
  "${BINARY}" keys show "${KEY}" -a --keyring-backend "${KEYRING_BACKEND}"
}

# ── 트랜잭션 인클루전 대기 ───────────────────────────────────────────────────
# wait_for_tx <hash> [max_tries]
wait_for_tx() {
  local hash="$1" tries="${2:-45}" out
  for ((i = 1; i <= tries; i++)); do
    if out="$("${BINARY}" query tx "${hash}" --node "${NODE}" --output json 2>/dev/null)"; then
      if [ -n "${out}" ] && [ "$(jq -r '.txhash // empty' <<<"${out}")" = "${hash}" ]; then
        echo "${out}"; return 0
      fi
    fi
    sleep 2
  done
  die "tx ${hash} 가 $((tries * 2))초 내 블록에 포함되지 않았습니다."
}

# ── 브로드캐스트: 서브커맨드 실행 → 인클루전 대기 → 성공 tx JSON 을 stdout 출력 ─
# broadcast <tx subcommand...>   (TX_FLAGS 자동 추가)
# 부수효과: 성공 시 전역 TX_HASH / TX_HEIGHT 설정 (호출자는 재파싱 불필요).
broadcast() {
  local resp hash raw_code txjson code
  resp="$("${BINARY}" "$@" "${TX_FLAGS[@]}")" || die "브로드캐스트 실패: $*"
  hash="$(jq -r '.txhash // empty' <<<"${resp}")"
  [ -n "${hash}" ] || die "txhash 파싱 실패: ${resp}"
  raw_code="$(jq -r '.code // 0' <<<"${resp}")"
  [ "${raw_code}" = "0" ] || die "tx 거부 (code ${raw_code}): $(jq -r '.raw_log // .' <<<"${resp}")"
  log "txhash=${hash} 브로드캐스트됨. 블록 포함 대기..."
  txjson="$(wait_for_tx "${hash}")"
  code="$(jq -r '.code // 0' <<<"${txjson}")"
  [ "${code}" = "0" ] || die "tx 실패 (code ${code}): $(jq -r '.raw_log // .' <<<"${txjson}")"
  read -r TX_HASH TX_HEIGHT < <(jq -r '"\(.txhash) \(.height)"' <<<"${txjson}")
  echo "${txjson}"
}

# ── 스마트 쿼리: 컨트랙트 state 조회 → .data 반환 ────────────────────────────
# query_smart <contract> <query_json>
# 쿼리 실패나 빈/null .data 는 die (호출자가 garbage 값을 소비하지 않도록).
# 반드시 명령치환 `x="$(query_smart ...)"` 로 받을 것 — 그래야 die(exit) 가
# set -e 로 호출자까지 전파됨(프로세스치환 `<(...)` 은 전파 안 됨).
query_smart() {
  local out
  out="$("${BINARY}" query wasm contract-state smart "$1" "$2" \
    --node "${NODE}" --output json | jq '.data')" || die "스마트 쿼리 실패: $1 ${2}"
  [ -n "${out}" ] && [ "${out}" != "null" ] || die "스마트 쿼리 결과 없음(null): $1 ${2}"
  printf '%s\n' "${out}"
}

# ── 이벤트 속성 추출 ─────────────────────────────────────────────────────────
# tx_attr <txjson> <event_type> <attr_key>   (동일 키 여러 개면 마지막 값)
tx_attr() {
  jq -r --arg t "$2" --arg k "$3" '
    [ .events[]? | select(.type==$t) | .attributes[]?
      | select((.key|tostring)==$k) | .value ] | last // empty' <<<"$1"
}

# ── 배포 기록 (deployments/<chain>.json) ─────────────────────────────────────
init_record() {
  mkdir -p "${DEPLOY_DIR}"
  if [ ! -f "${RECORD_FILE}" ]; then
    jq -n --arg cid "${CHAIN_ID}" --arg bin "${BINARY}" '{
      chain_id: $cid,
      binary: $bin,
      updated_at: (now | todate),
      forwarder: { code_id: null, code_history: [] },
      factory: {},
      forwarders: []
    }' >"${RECORD_FILE}"
    log "배포 기록 생성: ${RECORD_FILE}"
  fi
}

# record_update '<jq filter>' [jq args...]   (in-place, updated_at 자동 갱신)
record_update() {
  local filter="$1"; shift
  init_record
  local tmp; tmp="$(mktemp)"
  jq "$@" "(${filter}) | .updated_at = (now | todate)" "${RECORD_FILE}" >"${tmp}" \
    && mv "${tmp}" "${RECORD_FILE}"
}

record_get() { jq -r "$1 // empty" "${RECORD_FILE}" 2>/dev/null || true; }

# ── store_code: wasm 업로드 → 전역 CODE_ID/STORE_TX_HASH/STORE_HEIGHT 설정 ────
store_code() {
  local wasm="$1"
  [ -f "${wasm}" ] || die "wasm 파일 없음: ${wasm} (먼저 scripts/build.sh 실행)"
  log "store: ${wasm}"
  local tmp txjson
  tmp="$(mktemp)"
  broadcast tx wasm store "${wasm}" >"${tmp}"
  txjson="$(cat "${tmp}")"
  rm -f "${tmp}"
  CODE_ID="$(tx_attr "${txjson}" store_code code_id)"
  [ -n "${CODE_ID}" ] || die "code_id 파싱 실패"
  STORE_TX_HASH="${TX_HASH}"
  STORE_HEIGHT="${TX_HEIGHT}"
  log "→ code_id=${CODE_ID} tx=${STORE_TX_HASH} height=${STORE_HEIGHT}"
}

# ── 표 형태 요약 출력 ────────────────────────────────────────────────────────
print_record() {
  [ -f "${RECORD_FILE}" ] || die "배포 기록 없음: ${RECORD_FILE}"
  jq . "${RECORD_FILE}"
}
