# Skip Go CosmWasm Contracts 분석

> 참조 레포: `github.com/skip-mev/skip-go-cosmwasm-contracts`  
> 분석 일자: 2026-06-09

---

## 아키텍처 개요

```
호출자 (Relayer)
    │
    ▼
Entry Point Contract          ← 진입점. 스왑·전송 로직 오케스트레이션
    │
    ├── IBC Adapter (ibc-hooks)      ← IBC 전송 (ibc-hooks 방식)
    │       └── Sudo callback (ACK/Timeout)
    │
    └── IBC Adapter (ibc-callbacks)  ← IBC 전송 (ICS-29 callbacks 방식)
```

### 역할 분리

| 컨트랙트                | 역할                                             |
| ----------------------- | ------------------------------------------------ |
| `entry-point`           | 스왑 + IBC 전송 오케스트레이션. 검증·라우팅 담당 |
| `ibc-hooks adapter`     | `ibc-hooks` 모듈을 통한 IBC 전송 실행            |
| `ibc-callbacks adapter` | ICS-29 callbacks를 통한 IBC 전송 실행            |

---

## 컨트랙트별 상세

### 1. Entry Point (`contracts/entry-point`)

#### Instantiate

```json
{
  "swap_venues": [],
  "ibc_transfer_contract_address": "<IBC_ADAPTER_ADDRESS>",
  "hyperlane_transfer_contract_address": null
}
```

#### Execute 메시지

| 메시지                     | 설명                                        |
| -------------------------- | ------------------------------------------- |
| `Action`                   | 스왑 없이 바로 액션(전송) 실행              |
| `ActionWithRecover`        | Action + 실패 시 recover_addr로 환불        |
| `SwapAndAction`            | 스왑 후 액션 실행                           |
| `SwapAndActionWithRecover` | SwapAndAction + 실패 시 recover_addr로 환불 |
| `UserSwap`                 | 내부 스왑 처리 (SubMsg reply 경유)          |
| `PostSwapAction`           | 스왑 완료 후 액션 실행 (SubMsg reply 경유)  |

#### Action 타입

```rust
pub enum Action {
    Transfer { to_address: String },          // 동일 체인 전송
    IbcTransfer { ibc_info: IbcInfo, ... },   // IBC 전송
    ContractCall { contract_address, msg },    // 컨트랙트 호출
}
```

#### IbcInfo 구조

```rust
pub struct IbcInfo {
    pub source_channel: String,   // Injective 측 IBC 채널 (예: "channel-23")
    pub receiver: String,         // 목적지 체인 수신 주소 (예: "cosmos1...")
    pub fee: Option<IbcFee>,      // Neutron 전용 ICS-29 수수료
    pub memo: String,             // IBC packet memo
    pub recover_address: String,  // ⚠️ 반드시 현재 체인(Injective) 주소여야 함
    pub encoding: Option<String>,
    pub eureka_fee: Option<EurekaFee>,
}
```

---

## ⚠️ 핵심 제약: `ibc_info.recover_address`

Entry Point의 `validate_and_dispatch_action` 함수(execute.rs:705)에서:

```rust
deps.api.addr_validate(&ibc_info.recover_address)?;
```

**`recover_address`는 Entry Point가 배포된 체인의 bech32 prefix를 따라야 합니다.**

| 체인            | 올바른 prefix | 잘못된 예       |
| --------------- | ------------- | --------------- |
| Injective (inj) | `inj1...`     | `cosmos1...` ❌ |
| Osmosis (osmo)  | `osmo1...`    | `inj1...` ❌    |
| Cosmos Hub      | `cosmos1...`  | `inj1...` ❌    |

### 오류 메시지

```
Generic error: addr_validate errored: invalid Bech32 prefix; expected inj, got cosmos: execute wasm contract failed
```

### 해결 방법

`ibc_info.recover_address`에는 **Injective 주소(`inj1...`)** 를 사용해야 합니다.  
`ibc_info.receiver`는 목적지 체인 주소이므로 `cosmos1...` 등 다른 prefix 사용 가능합니다.

```json
{
  "action_with_recover": {
    "action": {
      "ibc_transfer": {
        "ibc_info": {
          "source_channel": "channel-23",
          "receiver": "cosmos1vx4h3qns352fjasa83mesadcf3jwn3f2duptxt",   ✅ 목적지 체인 주소
          "recover_address": "inj1d3dgs97klc8un9nr5f9f03yr959p3uchzv9agw"  ✅ 반드시 inj1 주소
        }
      }
    },
    "recovery_addr": "inj1d3dgs97klc8un9nr5f9f03yr959p3uchzv9agw"         ✅ 반드시 inj1 주소
  }
}
```

---

## IBC Adapter — ibc-hooks (`contracts/adapters/ibc/ibc-hooks`)

### 동작 원리

1. Entry Point가 `IbcTransfer` 메시지를 IBC Adapter로 전송
2. Adapter가 Cosmos SDK `MsgTransfer` 실행 (이때 `memo`에 ibc-hooks 정보 포함)
3. 목적지 체인에서 ibc-hooks 미들웨어가 memo를 파싱해 컨트랙트 호출
4. ACK/Timeout은 `sudo` 콜백으로 Adapter에 전달
5. Timeout/실패 시 `recover_address`로 환불 처리

### 상태 저장

```rust
IN_PROGRESS_RECOVER_ADDRESS  // 진행 중인 전송의 recover 주소 임시 저장
IN_PROGRESS_CHANNEL_ID       // 진행 중인 채널 ID 임시 저장
ACK_ID_TO_RECOVER_ADDRESS    // (channel, sequence) → recover_address 매핑
```

### Sudo 콜백 처리

```
IbcAck(success=true)  → 상태 정리
IbcAck(success=false) → recover_address로 환불
IbcTimeout            → recover_address로 환불
```

---

## IBC Adapter — ibc-callbacks (`contracts/adapters/ibc/ibc-callbacks`)

ibc-hooks와 구조 동일하지만 ICS-29 packet callbacks 방식을 사용합니다.  
Neutron 등 callbacks 모듈을 지원하는 체인에서 사용합니다.

---

## timeout_timestamp 단위

**나노초(ns)** 를 사용합니다. Unix 시간(초)이 아닙니다.

```bash
# 현재 시각 + 10분 (나노초)
TIMEOUT_NS=$(( $(date +%s) * 1000000000 + 600000000000 ))
```

| 값                    | 해석                             |
| --------------------- | -------------------------------- |
| `1749470600000000000` | 2025-06-09 (정상)                |
| `1781509684000000`    | 1970-01-20 (과거 → 즉시 만료) ❌ |

---

## hook_data 필드 요약 (`action_with_recover`)

```json
{
  "action_with_recover": {
    "sent_asset": {
      "native": {
        "denom": "<토큰 denom>",   // ERC20이면 "erc20:0x..."
        "amount": "<문자열 정수>"
      }
    },
    "timeout_timestamp": <나노초 Unix 시간>,
    "action": {
      "ibc_transfer": {
        "ibc_info": {
          "source_channel": "<Injective IBC 채널>",
          "receiver": "<목적지 체인 주소>",  // 목적지 체인 prefix 사용 가능
          "fee": null,
          "memo": "",
          "recover_address": "<inj1... 주소>",  // ⚠️ 반드시 Injective 주소
          "encoding": null,
          "eureka_fee": null
        },
        "fee_swap": null
      }
    },
    "exact_out": false,
    "min_asset": {
      "native": {
        "denom": "<목적지 denom>",
        "amount": "<최소 수령 금액 (슬리피지 허용치)>"
      }
    },
    "recovery_addr": "<inj1... 주소>"  // ⚠️ 반드시 Injective 주소
  }
}
```

---

## 관련 파일

| 파일                                                       | 내용                                           |
| ---------------------------------------------------------- | ---------------------------------------------- |
| `contracts/forwarder/test-data/ibc-transfer-hookdata.json` | 실제 테스트용 hook_data                        |
| `scripts/transfer_call.sh`                                 | TransferCall 실행 스크립트 (timeout 자동 계산) |
| `docs/ibc-channel-status.md`                               | Injective 테스트넷 IBC 채널 상태               |
