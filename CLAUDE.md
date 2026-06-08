# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 이 저장소는 무엇인가

CCTP v2 **forwarder factory** 의 CosmWasm(Rust) 구현. `forwarder-factory` 컨트랙트가
`instantiate2` 로 라우트별 `forwarder` 컨트랙트를 결정론적으로 생성하고, 각 forwarder 는
브릿지된 USDC 를 받아 Skip 엔트리포인트를 통해 다음 목적지로 전달한다. EVM
`skip-go-evm-contracts/ForwarderFactory` 패턴을 모델로 한다.

## 툴체인 함정 (먼저 읽을 것)

- **`rust-toolchain.toml` 이 Rust `1.77.0` 으로 고정되어 있다.** 이 핀은 필수다: 머신 기본
  `stable` 은 Cargo 워크스페이스 의존성 상속을 못 하는 구버전 `1.62.1` 이다. 항상 핀이
  적용되게 둘 것(이 저장소에서 그냥 `cargo` 를 쓰면 1.77.0 으로 해석됨). **`cargo +stable`
  을 쓰지 말 것.**
- **`cargo-make` 를 쓰지 말 것.** 현재 릴리스는 Cargo ≥ 1.78 (lockfile v4 / `edition2024`
  의존성)을 요구하여 고정된 1.77.0 에서 설치 불가. 이 저장소의 태스크 러너는 **`just`**
  (`brew install just`) — 툴체인에 종속되지 않는 사전 빌드 바이너리다. `Makefile` 은 없다.
- `instantiate2` 는 `cosmwasm-std` 의 **`cosmwasm_1_2`** feature 를 요구한다(워크스페이스
  `Cargo.toml` 에 이미 설정됨). 이를 제거하면 `WasmMsg::Instantiate2`, `instantiate2_address`,
  `query_wasm_code_info` 가 깨진다.

## 자주 쓰는 명령

```bash
# 최적화된 wasm 빌드 → artifacts/forwarder.wasm, artifacts/forwarder_factory.wasm
just build                  # = scripts/build.sh (cargo build --lib + wasm-opt -Oz); binaryen 필요

just test                   # 전체 11개 테스트 (forwarder 단위 + factory E2E)
just ci                     # fmt-check + clippy(-D warnings) + test
just schema                 # contracts/*/schema/ 재생성
just release                # ci + build + schema

# 추가 설치 없는 cargo 별칭 (.cargo/config.toml 참고)
cargo wasm                  # raw wasm 빌드 (wasm-opt 없음)
cargo unit-test             # cargo test --workspace --locked
cargo lint                  # clippy -D warnings

# 단일 테스트 실행 (cargo 가 이름 필터를 그대로 전달):
cargo test -p forwarder-factory --test integration create_forwarder_matches_predicted_address
cargo test -p forwarder transfer_call_requires_skip_relayer   # forwarder 단위 테스트
```

`just` 레시피는 cwd 와 무관하게 저장소 루트에서 실행된다. `just --list` 로 전체 확인.

## 아키텍처

### 세 개의 크레이트 (이름이 중요하다)

- `contracts/forwarder` → 패키지 `forwarder` → **`forwarder.wasm`**
- `contracts/forwarder-factory` → 패키지 `forwarder-factory` → **`forwarder_factory.wasm`**
- `packages/shared` → 패키지 `forwarder-factory-shared`

크레이트 이름은 `forwarder-factory` 이며 절대 `factory` 가 아니다. shared 크레이트는
**컨트랙트 간 JSON 의 단일 출처**(`QueryMsg`, `FactoryConfigResponse`,
`ForwarderInstantiateMsg`, `ForwarderInfo`)다. 두 컨트랙트가 이에 의존하므로
factory↔forwarder 와이어 포맷이 어긋날 수 없다. shared 타입을 바꾸면 양쪽이 그에 맞춰
재컴파일된다.

### 설정은 forwarder 가 아니라 factory 에 산다

forwarder 자신의 `Config` 는 **불변 라우트 정체성**(`factory`, `sender_addr`,
`recipient_addr`, `dest_chain`) 만 저장한다 — instantiate2 salt 에 구워지는 필드들이다.
운영 설정(`skip_relayer_addr`, `skip_entrypoint_addr`, `refund_addr`, `owner`)은 forwarder 에
**저장되지 않으며**, 런타임에 부모 factory 로부터 `query_wasm_smart`(`load_factory_config`)로
읽어온다. 따라서 factory 에 `UpdateConfig` 한 번이면 모든 forwarder 의 동작이 한꺼번에
바뀐다. factory 쿼리가 실패하면 forwarder 는 오래된 라우팅 파라미터로 전달하는 대신
**revert** 한다(`ContractError::FactoryQueryFailed`).

### 결정론적 주소 (핵심 보장)

`make_salt = sha256(len-prefixed sender || dest_chain || recipient)` (필드마다 4바이트 BE
길이 접두사를 붙여 concat 충돌을 방지). factory 는 생성 **전** 에 `query_wasm_code_info` +
`instantiate2_address` 로 forwarder 주소를 예측하므로, 호출자는 해당 주소를 미리 펀딩할 수
있다. 생성 reply (`reply_on_success` + `parse_instantiate_response_data`) 에서 factory 는
`actual == predicted` 를 단언한다(아니면 `AddressMismatch`). E2E 테스트
`create_forwarder_matches_predicted_address` 가 이 불변식을 고정한다.

### forwarder 요청 생명주기

forwarder 는 fire-and-forget 가 아니라 **요청 기반**이다. `TransferCall` 은 `mint_tx_hash`
키로(`REQUEST_ID_BY_MINT_TX_HASH` 가 중복 제거) `TransferRequest` 를 `RequestStatus`
(`Pending → Transfer | Fail → Refund | Abandon`)와 함께 기록한다. 다음 단계의 Skip 전송은
`SubMsg::reply_on_error` 이므로, 전달 실패 시 tx 를 중단하는 대신 요청을 `Fail` 로 뒤집어
이후 `Refund`/`Abandon` 을 가능하게 한다. `TransferCall` 은 skip-relayer 게이트,
`Refund`/`Abandon` 은 owner-또는-relayer 게이트다 — 모든 인가는 **factory 에서 조회한**
주소와 비교한다.

### 두 개의 reply 핸들러, 상반된 의도

- factory `reply`: `reply_on_success` — instantiate2 주소 일치를 확인하고 forwarder 기록을
  영속화한다.
- forwarder `reply`: `reply_on_error` — 실패한 다음 단계 전송을 잡아 요청을 실패로 표시한다.

## 배포 / 업그레이드 (`scripts/` + `just`)

`scripts/` 가 체인 운영 계층을 담고, `just` 레시피는 얇은 래퍼다. 먼저
`scripts/config.env.example` → `scripts/config.env` (gitignore 됨) 로 복사할 것.

```bash
just deploy                                   # forwarder + factory 코드 저장, factory instantiate
just create-forwarder <sender> <dest> <recip> # 라우트 instantiate2 (예측 주소 먼저 출력)
just upgrade-factory                          # 새 factory 코드 저장 → migrate
just upgrade-forwarder                        # 새 forwarder 코드 저장 → factory UpdateConfig{code_id}
just query config|list|addr|record
```

모든 tx 는 Tx 해시 / code_id / 컨트랙트 & forwarder 주소 / salt / 버전 이력을
**`deployments/<chain-id>.json`** 에 기록한다(누적 원장, 체인당 파일 1개).

**forwarder 업그레이드 제약:** 각 forwarder 의 admin 은 **factory 컨트랙트**이고, factory 에는
`MigrateForwarder` 실행 메시지가 없다. 따라서 `upgrade-forwarder` 는 *이후* forwarder 를 위해
`forwarder_code_id` 만 다시 가리킬 뿐 — 기존 forwarder 는 옛 코드를 유지하며, factory 에
`MigrateForwarder { address, new_code_id, msg }` 엔드포인트(`WasmMsg::Migrate` emit)를 먼저
추가하지 않으면 일괄 마이그레이션할 수 없다. factory 자체 업그레이드(`migrate`)는 영향받지
않는다.

## 테스트 노트

E2E 테스트(`contracts/forwarder-factory/tests/integration.rs`)는 `cw-multi-test` 에
**`cosmwasm_1_2`** feature 와 `MockApiBech32("cosmwasm")`, `MockAddressGenerator` 를 함께
사용한다 — 이것들이 없으면 `CodeInfo` 쿼리가 미구현 상태이고 instantiate2 bech32 주소가
정규화에 실패한다. forwarder 단위 테스트는 factory 의 `Config` 응답을
`MockQuerier::update_wasm` 으로 모킹한다(실제 컨트랙트 간 호출 없음).

## 미완성 항목

`contracts/forwarder/src/msg.rs` 의 `SkipEntrypointExecuteMsg` / `SkipTransferCallPayload` 는
실제 Skip 엔트리포인트 메시지 스펙이 나오기 전까지의 플레이스홀더(`TODO` 표시)다.

## 워크플로

이 저장소는 bkit PDCA 워크플로로 운영된다. 설계/계획은 `docs/02-design/` 와 `docs/01-plan/`
아래에 있다. 현재 feature 는 `ForwarderFactory` (Do 단계 완료). 이 프로젝트의 응답은 끝에
bkit Feature Usage 리포트 블록으로 마무리하는 것이 원칙이다.
