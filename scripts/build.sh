#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# 두 컨트랙트(forwarder, forwarder-factory)를 빌드 → wasm-opt 최적화 → artifacts/.
# (구 루트 cw-build 로직을 통합한 자립형 스크립트)
#
# wasmd v0.54 이하 일부 체인은 reference-types / bulk-memory feature 미지원이므로
# 두 feature 가 기본 비활성인 Rust 1.77.0 으로 빌드하고 wasm-opt 로 후처리합니다.
#
# 사용법: scripts/build.sh
# 의존: rustup, wasm-opt(binaryen)
# 환경변수: TOOLCHAIN(기본 1.77.0), MAX_WASM_SIZE_KB(기본 800)
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

TOOLCHAIN="${TOOLCHAIN:-1.77.0}"
MAX_WASM_SIZE_KB="${MAX_WASM_SIZE_KB:-800}"
ARTIFACTS_DIR="${ROOT_DIR}/artifacts"

log() { printf '\033[1;34m[build]\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31m[err]\033[0m %s\n' "$*" >&2; exit 1; }

command -v rustup   >/dev/null 2>&1 || die "rustup 필요"
command -v wasm-opt >/dev/null 2>&1 || die "wasm-opt(binaryen) 필요: brew install binaryen"
command -v jq       >/dev/null 2>&1 || die "jq 필요: brew install jq"

# ── 툴체인 / wasm32 target 준비 ──────────────────────────────────────────────
if ! rustup toolchain list | grep -q "^${TOOLCHAIN}"; then
  log "Rust ${TOOLCHAIN} 설치"
  rustup toolchain install "${TOOLCHAIN}" --profile minimal
fi
rustup target add wasm32-unknown-unknown --toolchain "${TOOLCHAIN}" >/dev/null 2>&1 || true

# ── 워크스페이스 target 디렉터리 (workspace-aware) ───────────────────────────
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  TARGET_BUILD_DIR="${CARGO_TARGET_DIR}"
else
  TARGET_BUILD_DIR="$(cargo +"${TOOLCHAIN}" metadata --format-version 1 --no-deps 2>/dev/null \
    | jq -r '.target_directory')"
fi
[[ -n "${TARGET_BUILD_DIR}" && "${TARGET_BUILD_DIR}" != null ]] || TARGET_BUILD_DIR="${ROOT_DIR}/target"
RELEASE_DIR="${TARGET_BUILD_DIR}/wasm32-unknown-unknown/release"

mkdir -p "${ARTIFACTS_DIR}"
log "toolchain=${TOOLCHAIN}  target_dir=${TARGET_BUILD_DIR}"

# ── 멤버별 빌드 + 최적화 ─────────────────────────────────────────────────────
# build_one <멤버 디렉터리> <crate 산출 파일명(언더스코어)>
build_one() {
  local member_dir="$1" crate="$2"
  log ">>> ${member_dir} → ${crate}.wasm"
  ( cd "${ROOT_DIR}/contracts/${member_dir}" \
    && cargo +"${TOOLCHAIN}" build --release --target wasm32-unknown-unknown --lib )

  local release_wasm="${RELEASE_DIR}/${crate}.wasm"
  [[ -f "${release_wasm}" ]] || die "빌드 산출물 없음: ${release_wasm}"

  local out="${ARTIFACTS_DIR}/${crate}.wasm"
  wasm-opt -Oz --strip-debug --strip-dwarf --strip-producers -o "${out}" "${release_wasm}"

  local sz; sz="$(wc -c < "${out}" | tr -d ' ')"
  log "    → artifacts/${crate}.wasm ($((sz / 1024))KB)"
  if (( sz > MAX_WASM_SIZE_KB * 1024 )); then
    log "    [경고] wasm 크기가 ${MAX_WASM_SIZE_KB}KB 를 초과했습니다 ($((sz / 1024))KB)"
  fi
}

build_one "forwarder"         "forwarder"
build_one "forwarder-factory" "forwarder_factory"

echo ""
echo "=== 빌드 완료 (Oz 최적화) ==="
ls -lh "${ARTIFACTS_DIR}"/*.wasm | awk '{print $5, $9}'
