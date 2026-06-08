# Design: ForwarderFactory

> Plan: [`docs/01-plan/features/ForwarderFactory.plan.md`](../../01-plan/features/ForwarderFactory.plan.md)
> 근거 분석: [`docs/forwarder-factory-wasm.md`](../../forwarder-factory-wasm.md)
> 현재 구현: `src/contract.rs`, `src/state.rs`, `src/msg.rs` (forwarder로 이식 대상)

CCTP v2 forward 단일 컨트랙트를 **`forwarder-factory` + `forwarder` 2-크레이트 Cargo workspace**로 재편한다.
Factory는 `instantiate2`로 라우트별 Forwarder를 결정론적으로 생성하고, 공유 인프라 설정을 단일 소유한다(Plan 선택지 B).

---

## 1. Workspace 레이아웃

```
cctp-v2-wasm/
├── Cargo.toml                      # [workspace] members + 공통 의존성/profile
├── contracts/
│   ├── forwarder/                  # 패키지명: forwarder          → forwarder.wasm
│   │   ├── Cargo.toml
│   │   └── src/{lib,contract,msg,state,error}.rs   # 현 src/ 이식 + 리팩터
│   └── forwarder-factory/          # 패키지명: forwarder-factory  → forwarder_factory.wasm
│       ├── Cargo.toml
│       └── src/{lib,contract,msg,state,error}.rs   # 신규
├── packages/
│   └── shared/                     # (선택) Factory QueryMsg::Config 타입 공유용 경량 크레이트
│       └── src/lib.rs
└── artifacts/                      # forwarder.wasm, forwarder_factory.wasm
```

- 루트 `Cargo.toml`:
  ```toml
  [workspace]
  members = ["contracts/forwarder", "contracts/forwarder-factory", "packages/shared"]
  resolver = "2"

  [workspace.dependencies]
  cosmwasm-std = { version = "1.5.8", features = ["staking", "stargate", "cosmwasm_1_2"] }
  cosmwasm-schema = "1.5.8"
  cw-storage-plus = "1.2.0"
  cw2 = "1.1.2"
  # sha2, hex, serde, serde_json, thiserror, schemars ...
  ```
- **`cosmwasm_1_2` feature 필수** — `instantiate2_address`, `WasmMsg::Instantiate2` 사용.
- `packages/shared`: Forwarder가 Factory를 query할 때 `FactoryConfigResponse` 타입을 양쪽이 공유하기 위함. 의존 방향은 forwarder → shared, factory → shared (factory가 직접 forwarder 크레이트를 의존하지 않게 해 빌드 결합 최소화).

> 결정: 타입 공유는 `packages/shared` 경량 크레이트로. (대안: forwarder가 factory의 msg를 직접 의존 → 순환/빌드결합 우려로 비채택)

## 2. Factory 컨트랙트

### 2.1 State (`forwarder-factory/src/state.rs`)

```rust
#[cw_serde]
pub struct FactoryConfig {
    pub owner: Addr,
    pub forwarder_code_id: u64,
    // 공유 인프라 설정 (모든 forwarder가 query로 참조)
    pub skip_relayer_addr: Addr,
    pub skip_entrypoint_addr: Addr,
    pub refund_addr: Addr,
}

#[cw_serde]
pub struct ForwarderRecord {
    pub address: Addr,
    pub sender_addr: String,
    pub recipient_addr: String,
    pub dest_chain: String,
    pub created_height: u64,
}

pub const CONFIG: Item<FactoryConfig> = Item::new("factory_config");
// salt(=route 식별 해시) → 생성된 forwarder 레코드. 존재 여부로 중복 차단.
pub const FORWARDERS: Map<&[u8], ForwarderRecord> = Map::new("forwarders");
// instantiate2 reply 상관용 임시 저장 (reply.id → 생성 컨텍스트)
pub const PENDING: Map<u64, PendingForwarder> = Map::new("pending");
pub const NEXT_REPLY_ID: Item<u64> = Item::new("next_reply_id");

#[cw_serde]
pub struct PendingForwarder {
    pub salt: Binary,
    pub sender_addr: String,
    pub recipient_addr: String,
    pub dest_chain: String,
    pub predicted: Addr,
}
```

> **registry 키 결정**: `FORWARDERS` 키 = `salt` 원시 바이트(`&[u8]`). salt 자체가 (sender,dest_chain,recipient)의 충돌 없는 해시이므로 복합 string 키보다 단순·결정적. 조회 편의를 위해 `ForwarderRecord`에 원본 필드 평문 보존.

### 2.2 Messages (`forwarder-factory/src/msg.rs`)

```rust
#[cw_serde]
pub struct InstantiateMsg {
    pub owner: Option<String>,        // None → info.sender
    pub forwarder_code_id: u64,
    pub skip_relayer_addr: String,
    pub skip_entrypoint_addr: String,
    pub refund_addr: String,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// 라우트별 forwarder를 instantiate2로 생성 (멱등: 이미 있으면 에러)
    CreateForwarder { sender_addr: String, recipient_addr: String, dest_chain: String },
    /// 공유 설정 일괄 변경 (owner-only). 부분 업데이트.
    UpdateConfig {
        owner: Option<String>,
        forwarder_code_id: Option<u64>,
        skip_relayer_addr: Option<String>,
        skip_entrypoint_addr: Option<String>,
        refund_addr: Option<String>,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(FactoryConfigResponse)]   // == shared::FactoryConfigResponse
    Config {},
    /// 생성 전 주소 예측 (EVM getForwarderAddress 패리티)
    #[returns(ForwarderAddressResponse)]
    ForwarderAddress { sender_addr: String, recipient_addr: String, dest_chain: String },
    #[returns(ForwardersResponse)]
    Forwarders { start_after: Option<Binary>, limit: Option<u32> },
}
```

> **정렬 주의**: `Forwarders` 페이지네이션 키는 salt(=sha256 해시) 바이트 오름차순이다. 따라서 목록 순서는 **의미를 갖지 않는다**(sender/chain 기준 정렬 아님). 운영자가 "특정 sender의 forwarder만 나열" 같은 조회가 필요해지면 보조 인덱스(예: `Map<(sender, dest_chain), salt>`)를 차기에 추가한다.

`FactoryConfigResponse`(shared)는 forwarder가 실행 시 읽는 공유값: `skip_relayer_addr`, `skip_entrypoint_addr`, `refund_addr`, `owner`, `forwarder_code_id`.

### 2.3 salt 알고리즘 (충돌 방지)

```rust
// 길이 프리픽스(u32 BE)로 필드 경계를 명확히 → concat 모호성 제거
fn make_salt(sender: &str, dest_chain: &str, recipient: &str) -> Binary {
    let mut h = Sha256::new();
    for part in [sender, dest_chain, recipient] {
        h.update((part.len() as u32).to_be_bytes());
        h.update(part.as_bytes());
    }
    Binary::from(h.finalize().to_vec())   // 32 bytes
}
```

> Plan에서 지적한 `("ab","c")` vs `("a","bc")` 충돌을, **각 필드 앞 4바이트 길이 프리픽스**로 차단. (도메인 구분자 대신 길이 프리픽스 채택 — 임의 문자 포함 입력에도 안전)

### 2.4 CreateForwarder 흐름

```
owner/anyone? ──CreateForwarder(sender,recipient,dest_chain)──► Factory
  1. salt = make_salt(...)
  2. FORWARDERS.has(salt)? → yes면 Err(ForwarderExists)  [중복 1차 차단]
  3. checksum = deps.querier.query_wasm_code_info(forwarder_code_id)?.checksum
  4. predicted = instantiate2_address(checksum, &canonical(env.contract.address), &salt)?
  5. PENDING.save(reply_id, {salt, fields, predicted})
  6. SubMsg::reply_on_success(
        WasmMsg::Instantiate2 {
          admin: Some(env.contract.address),   // ★ factory를 admin으로 → 향후 migrate(A) 대비
          code_id: forwarder_code_id,
          label: format!("forwarder-{}", hex(salt)),
          msg: forwarder::InstantiateMsg { factory, sender_addr, recipient_addr, dest_chain },
          funds: [],
          salt,
        }, reply_id)
  ── reply(success) ──►
  7. addr = parse_instantiate2_reply(msg) → predicted와 일치 assert(불일치 시 AddressMismatch)
  8. FORWARDERS.save(salt, ForwarderRecord{...})   [최종 등록]
  9. PENDING.remove(reply_id)
```

**reply 도입 근거 (feasibility doc 대비 의도적 분기)**
- `docs/forwarder-factory-wasm.md`는 "주소가 결정론적이라 reply 불필요"라고 본다. 본 설계는 **`reply_on_success`를 채택**하되, 그 목적은 오직 **예측 주소(predicted) == 실제 생성 주소를 검증(`AddressMismatch` assert)**하는 안전장치다. checksum/canonical addr 계산이 어긋나면 잘못된 주소를 registry에 영구 기록하는 사고를 방지한다.
- 만약 이 검증을 포기한다면 `PENDING`/`NEXT_REPLY_ID`/reply를 모두 제거하고, `execute` 단계에서 `predicted`를 곧바로 `FORWARDERS.save`하는 단순화가 가능하다(feasibility doc 방식). → **MVP는 검증 우선으로 reply 채택**, 가스/복잡도가 문제되면 차기에 단순화.

**reply 파싱 방법 (구현 명시 — 가장 실수 잦은 지점)**
```rust
use cosmwasm_std::{SubMsgResult, parse_instantiate_response_data};
// reply.id 로 PENDING 로드
let SubMsgResult::Ok(res) = msg.result else { return Err(Instantiate2Failed) };
let data = res.data.ok_or(Instantiate2Failed)?;            // MsgInstantiateContract2Response (protobuf)
let parsed = parse_instantiate_response_data(&data)?;       // cosmwasm-std 1.5 제공
let addr = deps.api.addr_validate(&parsed.contract_address)?;
ensure_eq!(addr, pending.predicted, ContractError::AddressMismatch);
```
> cosmwasm-std 1.5의 `parse_instantiate_response_data`(data 바이트 디코딩)를 사용한다. `MsgInstantiateContract2Response`도 `contract_address` 필드를 동일하게 노출하므로 동일 함수로 파싱 가능.

- **중복 2차 차단**: 같은 salt로 `Instantiate2` 재시도 시 체인(`wasmd`)이 주소 충돌로 자동 실패 → reply_on_success가 아니라 tx 전체 실패. 1차(step 2)는 명확한 에러 메시지 제공용.
- **권한**: `CreateForwarder`는 기본 **owner-only**로 시작(운영 통제). 추후 공개 생성 필요 시 완화. → 미해결 질문 ②와 함께 결정사항에 명시.

## 3. Forwarder 컨트랙트 (현 코드 리팩터)

### 3.1 State 변경 (`forwarder/src/state.rs`)

```rust
#[cw_serde]
pub struct Config {
    pub factory: Addr,         // ★ 신규: 공유설정 query 대상
    pub sender_addr: String,   // 라우트 식별 (salt 구성요소)
    pub recipient_addr: String,
    pub dest_chain: String,
    // ── 제거: refund_addr, skip_relayer_addr, skip_entrypoint_addr, owner (factory로 이전) ──
}
// RequestStatus / TransferRequest / NEXT_REQUEST_ID / REQUESTS_BY_ID /
// REQUEST_ID_BY_MINT_TX_HASH : 현행 그대로 유지
```

### 3.2 InstantiateMsg (factory가 동봉)

```rust
#[cw_serde]
pub struct InstantiateMsg {
    pub factory: String,        // 보통 instantiate 호출자(=factory)이지만 명시 전달
    pub sender_addr: String,
    pub recipient_addr: String,
    pub dest_chain: String,
}
```

> instantiate 시 `factory` = info.sender 와 일치하는지 검증(`factory == info.sender`)해, 임의 주소가 가짜 factory를 주입하지 못하게 한다.

### 3.3 실행 경로 변경 (공유값 query 치환)

현 `execute_transfer_call` / `execute_refund` / `execute_abandon`에서 `config.skip_*` / `refund_addr` / `owner` 참조를 **Factory query 결과**로 치환:

```rust
let fc: FactoryConfigResponse =
    deps.querier.query_wasm_smart(&config.factory, &FactoryQueryMsg::Config {})?;
// 권한: assert(info.sender == fc.skip_relayer_addr) 등
// forward 대상: fc.skip_entrypoint_addr
// refund 목적지: fc.refund_addr
// refund/abandon 권한: info.sender == fc.skip_relayer_addr || info.sender == fc.owner
```

- recipient_addr / dest_chain 은 **로컬 config** 사용(라우트 고정값).
- 그 외 request 저장·`reply_on_error` 실패 마킹·query(Requests/RequestByMintTxHash) 로직은 **현행 보존**(회귀 방지).
- **수용한 트레이드오프**: TransferCall/Refund/Abandon 매 호출마다 factory smart-query 1회가 추가되어 가스가 소폭 증가한다(공유값만 query, 라우트값은 로컬로 최소화). 또한 forwarder는 factory의 `Config` 스키마에 **강결합**된다 — factory가 삭제되거나 `FactoryConfigResponse` 스키마가 깨지면 전 forwarder의 forward가 멈춘다. 그래서 `packages/shared`로 타입을 단일화하고 factory migrate를 신중히 관리한다.

### 3.4 Forwarder 자체 reply (기존)

`reply_on_error`(forward 실패 시 `Fail` 마킹)는 현행 유지. id 네임스페이스가 factory와 무관(각 컨트랙트 독립)하므로 충돌 없음.

### 3.5 cw2 버저닝 & migrate 엔트리포인트 (양 크레이트)

`admin = factory`(선택지 A 경로 확보)가 실효를 가지려면 forwarder가 **지금부터** `migrate` 엔트리포인트와 cw2 버전 게이트를 갖춰야 한다(없으면 admin 지정이 무의미).

| 크레이트 | CONTRACT_NAME (cw2) | migrate |
|----------|---------------------|---------|
| forwarder | `crates.io:cctp-v2-forwarder` | **no-op 스텁 제공** (cw2 name/version 검증 + set_contract_version). 로직 변경 마이그레이션은 NG1(차기). |
| forwarder-factory | `crates.io:cctp-v2-forwarder-factory` | no-op 스텁(향후 factory 자체 업그레이드 대비). |

```rust
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    let ver = cw2::get_contract_version(deps.storage)?;
    ensure_eq!(ver.contract, CONTRACT_NAME, ContractError::InvalidMigration); // 다른 컨트랙트 차단
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?; // 버전 갱신
    Ok(Response::new().add_attribute("action", "migrate").add_attribute("version", CONTRACT_VERSION))
}
```
- `MigrateMsg`는 빈 구조체(`{}`)로 시작. forwarder의 기존 `CONTRACT_NAME`(`crates.io:cctp-v2-forward-contract`)은 신규 이름으로 교체 → **신규 배포 전제**(NG3, 기존 인스턴스 마이그레이션 미지원).

## 4. 시퀀스 요약

```
[배포]   Factory instantiate (forwarder_code_id, 공유설정)
[등록]   Owner → Factory.CreateForwarder(route)  ──instantiate2──►  Forwarder 생성/registry
[예측]   Relayer → Factory.ForwarderAddress(route)  →  주소 미리 확보 → USDC 선송금
[forward] Skip relayer → Forwarder.TransferCall  ──query Factory.Config──►  Skip entrypoint로 forward
[설정변경] Owner → Factory.UpdateConfig(skip_entrypoint=...)  →  이후 모든 forwarder 즉시 반영
[refund] Owner/relayer → Forwarder.Refund/Abandon  ──query Factory.refund_addr──►  BankMsg::Send
```

## 5. 에러 타입

- **Forwarder**: 기존 `ContractError` 유지 + `UnauthorizedFactory`(instantiate factory 불일치), `FactoryQueryFailed`(공유설정 조회 실패 → tx revert).
- **Factory**: `Unauthorized`(owner 아님), `ForwarderExists`(salt 중복), `InvalidCodeId`, `Instantiate2Failed`, `AddressMismatch`(예측≠실제).

## 6. 빌드/배포

- **의존성 변경 (필수)**: factory 크레이트의 `cosmwasm-std`에 **`features = ["cosmwasm_1_2"]`** 추가 — `instantiate2_address`, `query_wasm_code_info`, `WasmMsg::Instantiate2`가 이 feature 게이트 뒤에 있다. (현 `Cargo.toml`에는 미포함)
- **스키마 생성**: 각 크레이트에 `src/bin/schema.rs`(또는 `examples/schema.rs`) + `cargo schema`로 독립 JSON 스키마 산출. `packages/shared`의 `FactoryConfigResponse`/`FactoryQueryMsg`를 양쪽에서 re-export 해 factory·forwarder 스키마가 어긋나지 않게 한다.
- `cw-build` / `deploy.sh`를 2개 산출물 빌드로 갱신. cosmwasm/optimizer(workspace-optimizer) 사용 권장 → `artifacts/forwarder.wasm`, `artifacts/forwarder_factory.wasm`.
- 배포 순서: ① forwarder store(code_id 확보) → ② factory instantiate(forwarder_code_id 주입) → ③ CreateForwarder.
- 모든 컨트랙트 instantiate에서 `cw2::set_contract_version` 호출(§3.5).

## 7. 미해결 질문 → 설계 결정

| Plan 미해결 질문 | 설계 결정 |
|------------------|-----------|
| Factory query 실패 fallback? | **fallback 없음** — 공유설정은 forward 필수값. query 실패 시 `FactoryQueryFailed`로 tx 전체 revert(안전 우선). |
| `forwarder_code_id` 변경 허용 범위 | `UpdateConfig`로 **owner-only 변경 허용**. 단 변경 후 생성되는 forwarder만 신규 code_id 사용 → **기존 주소 예측은 code_id별로 달라짐**을 문서 경고. 기존 forwarder 로직 교체(migrate)는 NG1(다음 사이클). |
| registry 키 인코딩 | `FORWARDERS` 키 = **salt 원시 바이트(`&[u8]`)**. 평문 (sender,recipient,dest_chain)은 값에 보존. list 정렬 = salt 바이트 오름차순. |
| 기존 owner 권한 합치기 | **factory.owner 단일 권한**으로 통합. forwarder는 owner를 보유하지 않고 factory.owner를 query로 위임. |

추가 결정:
- **admin = factory**로 instantiate2 → 코드 변경 없이도 향후 선택지 A(migrate) 활성화 경로 확보.
- **CreateForwarder 권한 = owner-only**(MVP). 공개 생성은 향후 확장.

## 8. 구현 순서 (Do 단계 체크리스트)

1. [ ] 루트 workspace `Cargo.toml`(factory에 `cosmwasm_1_2` feature) + `packages/shared`(FactoryConfigResponse, FactoryQueryMsg).
2. [ ] `contracts/forwarder`: 현 `src/` 이식 → Config에서 공유필드 제거, factory 필드 추가. cw2 이름 교체 + no-op `migrate` 스텁(§3.5).
3. [ ] forwarder 실행경로를 Factory.Config query로 치환 + instantiate factory 검증.
4. [ ] forwarder 기존 테스트 이식 + Factory query mock 추가하여 통과.
5. [ ] `contracts/forwarder-factory`: state/msg/error + cw2 + no-op `migrate`.
6. [ ] `make_salt`(길이 프리픽스) + `instantiate2_address` 예측 + `ForwarderAddress` query.
7. [ ] `CreateForwarder`(SubMsg + reply, `parse_instantiate_response_data`로 주소 파싱 + `AddressMismatch` assert) + `FORWARDERS`/`PENDING` + 중복 차단.
8. [ ] `UpdateConfig`(owner-only) + `Config`/`Forwarders` query.
9. [ ] 각 크레이트 `cargo schema` bin + 빌드/배포 스크립트 2-산출물 갱신.
10. [ ] cw-multi-test 통합: Factory→CreateForwarder→예측주소 일치→TransferCall E2E + 중복차단 + UpdateConfig 반영.

## 9. 검증 기준 (DoD, Plan §7 상속)

- [ ] 2개 wasm 산출물 빌드 성공.
- [ ] `ForwarderAddress` 예측 == `CreateForwarder` 실제 생성 주소.
- [ ] 동일 라우트 재생성 실패(`ForwarderExists` 또는 체인 충돌).
- [ ] TransferCall이 Factory 공유설정을 정확히 참조(권한/대상/refund).
- [ ] `UpdateConfig` 후 forwarder 동작에 즉시 반영(E2E).
- [ ] 기존 forward/refund/abandon/request 추적 회귀 없음.
- [ ] gap 분석 Match Rate ≥ 90%.

## 10. 리스크 재확인 (Plan §6 + 설계 신규)

| 리스크 | 완화 |
|--------|------|
| 매 TransferCall마다 cross-contract query 가스 | 공유값만 query, 라우트값 로컬. 빈번 시 캐싱은 차기. |
| reply에서 예측≠실제 주소 | `AddressMismatch` assert로 조기 실패. canonical addr/ checksum 일치 테스트. |
| factory admin 권한 오남용 | owner 키 관리 운영 가이드. admin=factory는 migrate 미구현 상태라 현재 무위험. |
| `query_wasm_code_info` 미지원 체인 | 타깃 체인(CW 1.2+) 사전 확인 — Plan 리스크와 동일. |
