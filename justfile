# ─────────────────────────────────────────────────────────────────────────────
# just 태스크 (https://github.com/casey/just) — Rust 친화 태스크 러너.
#
# 설치:   brew install just
# 실행:   just <task>          (예: just ci)
# 목록:   just --list
#
# 개발 작업(fmt/clippy/test/wasm/schema)은 just 로,
# 체인 배포/업그레이드는 scripts/*.sh 래퍼 레시피로 처리합니다.
# 단축 명령(cargo wasm/unit-test/lint)은 .cargo/config.toml 별칭으로도 가능합니다.
# ─────────────────────────────────────────────────────────────────────────────

# 태스크 목록
default:
    @just --list

# 코드 포맷 적용
fmt:
    cargo fmt --all

# 포맷 검사(CI)
fmt-check:
    cargo fmt --all -- --check

# clippy(경고를 에러로)
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# 워크스페이스 전체 테스트
test:
    cargo test --workspace --locked

# wasm 라이브러리 릴리스 빌드(최적화 전)
wasm:
    cargo build --release --target wasm32-unknown-unknown --lib

# 두 컨트랙트 최적화 빌드 → artifacts/*.wasm (cargo + wasm-opt)
build:
    ./scripts/build.sh

# build 별칭
optimize: build

# 두 컨트랙트 JSON 스키마 생성 → contracts/*/schema/
schema:
    cd contracts/forwarder && cargo run --quiet --bin forwarder-schema
    cd contracts/forwarder-factory && cargo run --quiet --bin forwarder-factory-schema
    @echo "스키마 생성 완료 → contracts/*/schema/"

# CI 게이트: 포맷 검사 + clippy + 테스트
ci: fmt-check clippy test

# 릴리스 산출물: 검증 + 최적화 빌드 + 스키마
release: ci build schema

# 빌드 산출물 정리
clean:
    cargo clean

# 최초 배포 (scripts/deploy.sh)
deploy *args:
    ./scripts/deploy.sh {{args}}

# forwarder 생성: <sender> <dest_chain> <recipient>
create-forwarder sender dest recipient:
    ./scripts/create_forwarder.sh {{sender}} {{dest}} {{recipient}}

# factory 코드 업그레이드(migrate)
upgrade-factory *args:
    ./scripts/upgrade_factory.sh {{args}}

# forwarder 코드 업그레이드(UpdateConfig)
upgrade-forwarder:
    ./scripts/upgrade_forwarder.sh

# forwarder 예측 주소 조회: <sender> <dest_chain> <recipient>
predict-forwarder sender dest recipient:
    ./scripts/query.sh addr {{sender}} {{dest}} {{recipient}}

# 조회: config | list | addr | record [...]
query *args:
    ./scripts/query.sh {{args}}
