# Plan: ForwarderFactory

> 현재 단일 CCTP v2 forward 컨트랙트를 EVM `ForwarderFactory` 패턴에 대응하는
> **ForwarderFactory + Forwarder 2-크레이트 구조**로 재편한다.
> (문서 내 "Factory"는 모두 `ForwarderFactory` 컨트랙트를 가리킨다.)
> 근거 분석: [`docs/forwarder-factory-wasm.md`](../../forwarder-factory-wasm.md)

## Executive Summary

| 관점 | 내용 |
|------|------|
| **Problem (문제)** | 라우트(보낸이·목적체인·수취인)마다 컨트랙트를 사람이 직접 instantiate하고 주소를 사후에 알아야 해서, 사전 자금 송금·중복 배포 방지·일괄 설정 변경이 어렵다. |
| **Solution (해결책)** | `instantiate2` 기반 Factory를 도입해 라우트별 Forwarder를 결정론적으로 생성·주소 예측하고, 공유 인프라 설정(skip relayer/entrypoint/refund/owner)은 Factory가 단일 소유해 forwarder가 query로 참조한다(EVM Beacon 의도의 config-share 대체). |
| **Function UX Effect (기능 효과)** | 릴레이어는 배포 전에 forwarder 주소를 예측해 선송금 가능, 같은 라우트 재배포는 체인이 자동 차단, 공유 설정은 Factory state 한 번 수정으로 전체 일괄 변경. |
| **Core Value (핵심 가치)** | EVM ForwarderFactory와 1:1 운영 패리티를 확보하면서, CosmWasm의 원자적 init 장점으로 더 단순하고 안전한 멀티-라우트 forward 인프라. |

---

## 1. 배경 / 현재 아키텍처

현재는 단일 크레이트 `cctp-v2-forward-contract` 하나가 **라우트 1개 = 컨트랙트 1개**를 담당한다.

- `Config`(state.rs)가 **라우트 식별값**과 **공유 인프라값**을 한 구조체에 섞어 보유:
  - 라우트 식별: `sender_addr`, `recipient_addr`, `dest_chain`
  - 공유 인프라: `refund_addr`, `skip_relayer_addr`, `skip_entrypoint_addr`, `owner`
- `execute_transfer_call` → Skip entrypoint로 forward, `REQUESTS_BY_ID` / `REQUEST_ID_BY_MINT_TX_HASH`로 요청 상태 추적.
- `refund` / `abandon`으로 상태 전이, `reply_on_error`로 forward 실패 시 `Fail` 마킹.

**한계**: 라우트가 늘면 운영자가 매번 수동 instantiate + 주소 사후 확인. 공유 설정을 바꾸려면 모든 컨트랙트를 개별 수정. 동일 라우트 중복 배포를 막을 장치 없음.

## 2. 목표 / 비목표

### 목표 (이번 사이클)
- **G1**: Cargo workspace로 `forwarder-factory` / `forwarder` 2개 크레이트 분리, 각각 독립 wasm/code_id 빌드.
- **G2**: Factory가 `WasmMsg::Instantiate2`로 라우트별 Forwarder를 결정론적 생성.
- **G3**: salt = `sha256(sender_addr ‖ dest_chain ‖ recipient_addr)` (EVM keccak(sender,domain,mintRecipient) 패리티).
- **G4**: 생성 전 주소 예측 query(`instantiate2_address`) 제공 → 릴레이어 선송금 지원 (EVM `getForwarderAddress` 패리티).
- **G5**: 동일 라우트 재배포 시 체인 자동 실패로 중복 방지(자체 registry Map 병행).
- **G6**: 공유 인프라 설정(`skip_relayer_addr`, `skip_entrypoint_addr`, `refund_addr`, `owner`)을 **Factory 단일 소유** → Forwarder가 실행 시 Factory에 query (선택지 B).
- **G7**: 기존 forward / refund / abandon / request 추적 동작은 Forwarder에서 그대로 보존(회귀 없음).

### 비목표 (다음 사이클로 미룸)
- **NG1**: Migrate 기반 일괄 로직 업그레이드(선택지 A). admin=factory 지정만 해두고 migrate 실행 로직은 미구현.
- **NG2**: `destination_channel`을 라우트 키로 승격(현재는 transfer 호출 인자로 유지).
- **NG3**: 기존 배포 단일 컨트랙트의 자동 마이그레이션 — 신규 구조는 새 배포로 시작.
- **NG4**: Skip Entrypoint payload 정합성(코드 내 기존 TODO) — 본 작업 범위 밖, 기존 동작 유지.

## 3. 타깃 아키텍처

```
                ┌─────────────────────────────┐
                │          Factory            │
                │  state: SharedConfig        │   ← skip_relayer, skip_entrypoint,
                │         forwarder_code_id   │      refund, owner (공유 설정 단일 소유)
                │         FORWARDERS: Map      │      route → addr registry
                │                             │
                │  exec: CreateForwarder      │── Instantiate2(salt, ForwarderInit) ──┐
                │  exec: UpdateConfig (owner)  │                                       │
                │  query: Config              │                                       ▼
                │  query: ForwarderAddress     │                          ┌──────────────────────┐
                │  query: Forwarders (list)    │◄── query Config ─────────│       Forwarder      │
                └─────────────────────────────┘                          │ state: factory_addr  │
                                                                          │        route(sender, │
                                                                          │        recipient,    │
                                                                          │        dest_chain)   │
                                                                          │        requests...   │
                                                                          │ exec: TransferCall   │
                                                                          │ exec: Refund/Abandon │
                                                                          └──────────────────────┘
```

### 3.1 Config 분리 (선택지 B 핵심)

| 필드 | 소유 위치 | 비고 |
|------|----------|------|
| `sender_addr` | Forwarder (init, salt) | 라우트 식별 |
| `recipient_addr` | Forwarder (init, salt) | 라우트 식별 |
| `dest_chain` | Forwarder (init, salt) | 라우트 식별 |
| `skip_relayer_addr` | **Factory (공유)** | forward 권한 검증 시 query |
| `skip_entrypoint_addr` | **Factory (공유)** | forward 대상, query |
| `refund_addr` | **Factory (공유)** | refund 목적지, query |
| `owner` | **Factory (공유)** | refund/abandon 권한, config 변경 |
| `forwarder_code_id` | Factory | instantiate2 대상 code_id (주소 안정성 위해 고정) |

> Forwarder는 `factory_addr` 1개만 init으로 받고, 실행 시점에 Factory.`Config`를 query해 공유값을 읽는다.
> 공유값 변경 = Factory state 1회 수정으로 전 forwarder 일괄 반영 (Beacon shared-immutable 일괄 변경 의도).

### 3.2 주소 결정론 (address stability)

- `instantiate2_address(checksum, creator, salt)` — checksum은 `forwarder_code_id`의 코드 체크섬.
- **forwarder_code_id를 고정**해야 salt별 주소 예측이 안정적. code_id 교체는 NG1(다음 사이클)로 분리.
- salt 구성: `make_salt(sender, dest_chain, recipient) = sha256(sender.bytes ‖ dest_chain.bytes ‖ recipient.bytes)`.
  - 구분자/길이 prefix를 넣어 concatenation 충돌(예: ("ab","c") vs ("a","bc")) 방지 검토 → 길이 프리픽스 권장.

## 4. 메시지 스펙 (초안)

### Factory
```rust
InstantiateMsg { owner: Option<String>, forwarder_code_id: u64,
                 skip_relayer_addr, skip_entrypoint_addr, refund_addr: String }

ExecuteMsg::CreateForwarder { sender_addr, recipient_addr, dest_chain }  // instantiate2
ExecuteMsg::UpdateConfig { skip_relayer_addr?, skip_entrypoint_addr?, refund_addr?, owner?, forwarder_code_id? } // owner-only

QueryMsg::Config {}                                   // 공유 설정 반환 (forwarder가 호출)
QueryMsg::ForwarderAddress { sender_addr, recipient_addr, dest_chain } // 예측 주소
QueryMsg::Forwarders { start_after?, limit? }         // registry 목록
```

### Forwarder
```rust
InstantiateMsg { factory: String, sender_addr, recipient_addr, dest_chain }  // factory가 동봉

ExecuteMsg::TransferCall { mint_tx_hash, transfer_coin, hook_data, destination_channel } // 기존과 동일
ExecuteMsg::Refund { mint_tx_hash }
ExecuteMsg::Abandon { mint_tx_hash }

QueryMsg::Config {}            // 자기 라우트 + factory 주소
QueryMsg::Requests { ... }    // 기존 유지
QueryMsg::RequestByMintTxHash { mint_tx_hash }
```

실행 흐름 변화(TransferCall): `assert_skip_relayer` / forward 대상 / refund 목적지를 **Factory.Config query 결과**로 대체. 그 외 request 저장/리플라이 로직은 현행 유지.

## 5. 작업 분해 (Milestones)

- **M1 — Workspace 골격**: 루트 `Cargo.toml` workspace화, `contracts/forwarder`(현 코드 이동) + `contracts/forwarder-factory`(신규, 패키지명 `forwarder-factory`) 생성, 공통 의존성 정리. `cosmwasm_1_2` feature 활성화.
- **M2 — Forwarder 리팩터링**: `Config`에서 공유필드 제거 → `factory_addr` + route 식별값만 보유. TransferCall/Refund/Abandon이 Factory.Config를 query하도록 변경. 기존 테스트가 factory query mock과 함께 통과하도록 수정.
- **M3 — Factory 구현**: `SharedConfig`/`forwarder_code_id`/`FORWARDERS` Map state, `CreateForwarder`(instantiate2 + salt + 예측 + registry 저장), `UpdateConfig`, `Config`/`ForwarderAddress`/`Forwarders` query.
- **M4 — salt/주소 유틸 + 중복 방지**: `make_salt` (길이 프리픽스), `instantiate2_address` 예측, registry 중복 체크 + 체인 자동 실패 확인.
- **M5 — 빌드/배포 스크립트**: 두 wasm 빌드(`cw-build`/`deploy.sh` 갱신), factory 먼저 배포 → forwarder code_id 등록 흐름 문서화.
- **M6 — 테스트**: 유닛(salt 결정성, 예측=실제 주소, 중복 차단, 공유 config query 경로) + cw-multi-test 통합(Factory→Forwarder→TransferCall 엔드투엔드).

## 6. 리스크 & 완화

| 리스크 | 영향 | 완화 |
|--------|------|------|
| 매 실행마다 Factory query 추가 | 가스/지연 증가 | 공유값만 query, 라우트값은 로컬 보유. 필요 시 캐싱은 다음 사이클. |
| `forwarder_code_id` 교체 시 신규 주소 예측 변동 | 선송금 주소 불일치 | code_id 고정 정책 문서화, 로직 변경은 NG1(migrate)로 분리. |
| salt concatenation 충돌 | 서로 다른 라우트가 같은 주소 | 길이 프리픽스 또는 도메인 구분자 포함. |
| 체인의 `MsgInstantiateContract2` 미지원 | Factory 생성 실패 | 타깃 체인(wasmd 0.29+/CW1.2+) 사전 확인 (Injective 등). |
| 기존 단일 컨트랙트 운영 자산 | 마이그레이션 경로 부재 | NG3 — 신규 배포로 시작, 기존은 그대로 종료 운영. |

## 7. 검증 기준 (Definition of Done)

- [ ] `cargo build` / wasm 빌드가 factory·forwarder 2개 산출물 생성.
- [ ] `ForwarderAddress` query 예측 주소 == `CreateForwarder` 실제 생성 주소 (통합 테스트).
- [ ] 동일 (sender, dest_chain, recipient) 재생성 시 실패.
- [ ] Forwarder TransferCall이 Factory 공유 config를 정확히 참조(권한 검증·forward 대상·refund 목적지).
- [ ] 기존 forward/refund/abandon/request 추적 회귀 없음(이식된 테스트 통과).
- [ ] gap 분석 Match Rate ≥ 90%.

## 8. 미해결 질문 (Design 단계에서 확정)

- Factory query 실패(예: factory paused) 시 Forwarder fallback 정책?
- `UpdateConfig`로 `forwarder_code_id` 변경 허용 범위(향후 migrate 사이클과의 경계).
- registry 키 인코딩(salt hex vs (sender,chain,recipient) 복합키) 및 list 페이지네이션 정렬.
- 기존 단일 컨트랙트의 `owner` 기반 권한을 factory.owner로 합칠 때 운영 키 관리.

---

### 결정 사항 (사용자 확정)
1. **Repo**: Cargo workspace 2-crate (`forwarder-factory` + `forwarder`).
2. **업그레이드 전략**: B — 공유 config를 Factory가 소유, Forwarder가 query (A/migrate는 다음 사이클).
3. **Salt 키**: `sender_addr + dest_chain + recipient_addr` (EVM 패리티).
