# Forwarder Contract 개발 문서

## 1. 개요

`forwarder` 컨트랙트는 CCTP(Cross-Chain Transfer Protocol)를 통해 브릿지된 USDC를 수신하여, Skip EntryPoint 컨트랙트를 통해 지정된 목적지 체인으로 포워딩하는 CosmWasm 스마트 컨트랙트입니다.

각 `forwarder` 인스턴스는 하나의 라우트(source → destination)에 대응하며, `forwarder-factory`가 `instantiate2`를 통해 결정론적으로 생성합니다.

---

## 2. 아키텍처

### 2.1. 컨트랙트 구조

```
forwarder-factory
    └── forwarder (per-route, created via instantiate2)
            └── Skip EntryPoint (external)
```

### 2.2. 설정 분리 원칙

- **Forwarder 로컬 설정** (`CONFIG`): 라우트 고유 정보만 저장
  - `factory`: 부모 factory 컨트랙트 주소 (runtime에 설정 조회 대상)
  - `sender_addr`: 발신자 주소 (EVM 주소, salt에 구워짐)
  - `recipient_addr`: 최종 수신자 주소 (salt에 구워짐)
  - `dest_chain`: 목적지 체인 이름 (salt에 구워짐)

- **Factory 공유 설정** (runtime 조회): 운영 파라미터
  - `skip_relayer_addr`: TransferCall 호출 권한자
  - `skip_entrypoint_addr`: Skip EntryPoint 컨트랙트 주소
  - `owner`: 관리자 주소 (Refund/Abandon 권한자)

> Factory에서 단 한 번 `UpdateConfig`하면 모든 forwarder의 동작이 일괄 변경됩니다.

---

## 3. 상태 관리 (State)

### 3.1. Storage 항목

| 키                           | 타입                        | 설명                               |
| ---------------------------- | --------------------------- | ---------------------------------- |
| `config`                     | `Config`                    | 라우트 고유 설정                   |
| `next_request_id`            | `u64`                       | 다음 요청 ID (1부터 시작)          |
| `requests_by_id`             | `Map<u64, TransferRequest>` | 요청 ID → 요청 데이터              |
| `request_id_by_mint_tx_hash` | `Map<&str, u64>`            | mint_tx_hash → 요청 ID (중복 방지) |

### 3.2. TransferRequest 구조

```rust
pub struct TransferRequest {
    pub id: u64,
    pub mint_tx_hash: String,         // CCTP mint tx 해시 (고유 키)
    pub transfer_coin: Coin,          // 전송 자산
    pub source_tx_height: u64,        // 요청 생성 블록 높이
    pub source_tx_index: Option<u32>, // 트랜잭션 인덱스
    pub destination_channel: String,  // IBC 채널
    pub status: RequestStatus,        // 현재 상태
    pub hook_data: Option<String>,    // 원본 hook_data JSON 문자열
    pub error_msg: Option<String>,    // 실패 시 에러 메시지
}
```

### 3.3. 요청 상태 FSM (Finite State Machine)

```
[생성]
  │
  ▼
Pending ──(EntryPoint 호출 성공)──► Transfer  (최종 성공 상태)
  │
  └──(EntryPoint 호출 실패 / reply 수신)──► Fail
                                              │
                                ┌─────────────┴─────────────┐
                                ▼                           ▼
                             Refund ──────────────────► Abandon
                                                  (최종 종료 상태)
```

**상태 전이 규칙:**

- `Pending` → `Transfer`: EntryPoint SubMsg 성공 reply 수신
- `Pending` → `Fail`: EntryPoint SubMsg 실패 reply 수신
- `Fail` → `Refund`: `Refund` 실행 (owner 또는 skip_relayer만)
- `Fail` → `Abandon`: `Abandon` 실행 직접 가능 (refund 없이 포기)
- `Refund` → `Abandon`: `Abandon` 실행

> **삭제 없음**: 요청은 상태 변경만 하며, 상태에서 제거되지 않습니다.

---

## 4. 메시지 (Messages)

### 4.1. InstantiateMsg

Factory가 `instantiate2`로 호출합니다. `factory` 주소는 반드시 `info.sender`와 일치해야 합니다.

```json
{
  "factory": "cosmos1...",
  "sender_addr": "0x...",
  "recipient_addr": "osmo1...",
  "dest_chain": "osmosis-1"
}
```

### 4.2. ExecuteMsg

#### `TransferCall`

Skip Relayer만 호출 가능합니다.

```json
{
  "transfer_call": {
    "mint_tx_hash": "0xabc...",
    "transfer_coin": { "denom": "ibc/...", "amount": "1000000" },
    "hook_data": "{\"swap_and_action\":{...}}",
    "destination_channel": "channel-0"
  }
}
```

**처리 순서:**

1. `skip_relayer_addr` 권한 확인 (factory에서 조회)
2. 잔액 충분 여부 확인
3. `mint_tx_hash` 중복 확인
4. `hook_data` JSON 파싱 및 유효성 검증
5. hook_data 최상위 키 → Skip EntryPoint 함수 매핑
6. `post_swap_action` 또는 `action`에서 최종 수신자 주소 추출 → `config.recipient_addr`와 동일한지 검증
7. **TransferRequest STATE 등록 (상태: Pending)**
8. Skip EntryPoint 컨트랙트 호출 (`SubMsg::reply_always`)

**hook_data 최상위 키 매핑:**

| hook_data 최상위 키            | Skip EntryPoint 함수                   |
| ------------------------------ | -------------------------------------- |
| `swap_and_action`              | `ExecuteMsg::SwapAndAction`            |
| `swap_and_action_with_recover` | `ExecuteMsg::SwapAndActionWithRecover` |
| `action`                       | `ExecuteMsg::Action`                   |
| `action_with_recover`          | `ExecuteMsg::ActionWithRecover`        |

**hook_data 예시:**

```json
{
  "swap_and_action": {
    "user_swap": {
      "swap_exact_asset_in": {
        "swap_venue_name": "osmosis-poolmanager",
        "operations": [...]
      }
    },
    "min_asset": { "native": { "denom": "uosmo", "amount": "100" } },
    "timeout_timestamp": 1780905488938793000,
    "post_swap_action": {
      "transfer": { "to_address": "osmo1..." }
    },
    "affiliates": []
  }
}
```

#### `Refund`

Owner 또는 Skip Relayer만 호출 가능합니다.
**`Fail` 상태의 요청만 환불 가능합니다.**

```json
{
  "refund": {
    "mint_tx_hash": "0xabc..."
  }
}
```

**처리:**

1. 권한 확인
2. 요청 상태가 `Fail`인지 확인
3. 컨트랙트 잔액 충분 여부 확인
4. `config.sender_addr`로 자산 전송 (`BankMsg::Send`)
5. 요청 상태를 `Refund`로 변경 (삭제하지 않음)

> `sender_addr`가 EVM 주소(0x...)인 경우, Injective 체인에서는 `inj1...` 형식으로 변환이 필요합니다.

#### `Abandon`

Owner 또는 Skip Relayer만 호출 가능합니다.
**`Fail` 또는 `Refund` 상태의 요청만 포기 가능합니다.**

```json
{
  "abandon": {
    "mint_tx_hash": "0xabc..."
  }
}
```

**처리:**

1. 권한 확인
2. 요청 상태가 `Fail` 또는 `Refund`인지 확인
3. 요청 상태를 `Abandon`으로 변경 (삭제하지 않음)

### 4.3. QueryMsg

#### `Config`

```json
{ "config": {} }
```

**응답:** 로컬 Config + Factory에서 조회한 `owner` 포함

#### `Requests`

```json
{
  "requests": {
    "start_after": null,
    "limit": 20,
    "order": "asc",
    "status": "fail"
  }
}
```

- `status`: 선택적 필터 (`pending`, `transfer`, `fail`, `refund`, `abandon`)
- `order`: `asc` (기본) 또는 `desc`
- `limit`: 최대 100 (기본 20)

#### `RequestByMintTxHash`

```json
{
  "request_by_mint_tx_hash": {
    "mint_tx_hash": "0xabc..."
  }
}
```

---

## 5. Reply 처리

EntryPoint 호출 시 `SubMsg::reply_always`를 사용합니다.

| Reply 결과          | 처리                                            |
| ------------------- | ----------------------------------------------- |
| `SubMsgResult::Ok`  | 요청 상태를 `Transfer`로 업데이트               |
| `SubMsgResult::Err` | 요청 상태를 `Fail`로 업데이트, 에러 메시지 저장 |

---

## 6. 권한 구조

| 액션           | 권한                                   |
| -------------- | -------------------------------------- |
| `TransferCall` | `skip_relayer_addr` (factory에서 조회) |
| `Refund`       | `owner` 또는 `skip_relayer_addr`       |
| `Abandon`      | `owner` 또는 `skip_relayer_addr`       |

모든 권한 주소는 Factory 컨트랙트에서 런타임에 조회합니다. Factory 조회 실패 시 트랜잭션이 revert됩니다.

---

## 7. 에러 코드

| 에러                       | 설명                                       |
| -------------------------- | ------------------------------------------ |
| `Unauthorized`             | 권한 없는 주소의 호출                      |
| `FactoryMismatch`          | instantiate 시 factory ≠ sender            |
| `FactoryQueryFailed`       | Factory 설정 조회 실패                     |
| `NoFundsToForward`         | 전송 금액이 0                              |
| `DuplicateMintTxHash`      | 이미 처리된 mint_tx_hash                   |
| `InsufficientRequestFunds` | 컨트랙트 잔액 부족                         |
| `InvalidStatusTransition`  | 허용되지 않는 상태 전이                    |
| `RequestNotFound`          | 요청 없음                                  |
| `MemoEncodeError`          | hook_data JSON 파싱 실패                   |
| `InvalidHookData`          | 알 수 없는 hook_data 최상위 키             |
| `RecipientMismatch`        | hook_data의 수신자 ≠ config.recipient_addr |

---

## 8. 테스트 커버리지

| 테스트                                 | 검증 내용                                |
| -------------------------------------- | ---------------------------------------- |
| `instantiate_rejects_factory_mismatch` | factory ≠ sender 거부                    |
| `instantiate_and_query_config`         | 초기화 및 설정 조회                      |
| `transfer_call_requires_skip_relayer`  | skip_relayer 외 호출 거부                |
| `transfer_call_registers_and_forwards` | 정상 TransferCall 흐름, Pending→Transfer |
| `reply_marks_request_fail`             | EntryPoint 실패 시 Fail 상태로 전환      |
| `refund_and_abandon_flow`              | Refund → Abandon 전체 흐름               |
| `refund_rejected_for_transfer_status`  | Transfer 상태 요청 환불 거부             |
| `recipient_mismatch_rejected`          | hook_data 수신자 불일치 거부             |
