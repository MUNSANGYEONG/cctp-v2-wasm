# 배포 / 업그레이드 스크립트

cctp-v2 `forwarder-factory` + `forwarder` 컨트랙트의 빌드·배포·운영·업그레이드 자동화 스크립트.
모든 트랜잭션 결과(Tx 해시 · code_id · 컨트랙트 주소 · forwarder 주소)는
`deployments/<chain-id>.json` 에 누적 기록됩니다.

## 사전 준비

```bash
cp scripts/config.env.example scripts/config.env
# BINARY / CHAIN_ID / NODE / KEY / GAS_PRICES / SKIP_*_ADDR 등 작성
```

의존: `jq`, 체인 바이너리(`wasmd`/`injectived`/...), 빌드 시 `rustup` + `wasm-opt`(binaryen).

### 서명 키 등록

`config.env` 의 `KEY` 가 키링에 없으면 먼저 등록합니다(니모닉 복구 예시):

```bash
"$BINARY" keys add "$KEY" --recover --keyring-backend "$KEYRING_BACKEND"   # 니모닉 입력
"$BINARY" keys show "$KEY" -a --keyring-backend "$KEYRING_BACKEND"          # 주소 확인
```

## 워크플로

| 단계                 | 명령                                                            | 설명                                                                         |
| -------------------- | --------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| 빌드                 | `scripts/build.sh`                                              | 두 컨트랙트 → `artifacts/forwarder.wasm`, `artifacts/forwarder_factory.wasm` |
| 배포                 | `scripts/deploy.sh`                                             | forwarder store → factory store → factory instantiate                        |
| 생성                 | `scripts/create_forwarder.sh <sender> <dest_chain> <recipient>` | 라우트별 forwarder instantiate2 생성 (예측 주소 선조회)                      |
| 조회                 | `scripts/query.sh config\|list\|addr\|record`                   | factory 설정 / forwarder 목록 / 예측주소 / 로컬 기록                         |
| Factory 업그레이드   | `scripts/upgrade_factory.sh [migrate_msg]`                      | 새 factory store → `migrate` (admin 키 필요)                                 |
| Forwarder 업그레이드 | `scripts/upgrade_forwarder.sh`                                  | 새 forwarder store → factory `UpdateConfig{forwarder_code_id}`               |

## 배포 기록 (`deployments/<chain-id>.json`)

```jsonc
{
  "chain_id": "injective-888",
  "binary": "injectived",
  "updated_at": "2026-06-08T...",
  "forwarder": {
    "code_id": 124,
    "code_history": [
      { "code_id": 124, "store_tx": "...", "height": 123, "created_at": "..." },
    ],
  },
  "factory": {
    "code_id": 125,
    "address": "inj1...",
    "admin": "inj1...",
    "owner": "inj1...",
    "skip_relayer_addr": "...",
    "skip_entrypoint_addr": "...",
    "store_tx": "...",
    "instantiate_tx": "...",
    "instantiate_height": 456,
    "version_history": [
      {
        "code_id": 125,
        "action": "instantiate",
        "tx": "...",
        "height": 456,
        "at": "...",
      },
    ],
  },
  "forwarders": [
    {
      "sender_addr": "0x..",
      "dest_chain": "dydx-mainnet-1",
      "recipient_addr": "dydx1..",
      "address": "inj1..",
      "salt": "ab12..",
      "code_id": 124,
      "create_tx": "..",
      "height": 789,
      "created_at": "..",
    },
  ],
}
```

## 업그레이드 모델 (중요)

- **Factory**: instantiate 시 `--admin` 으로 지정한 키(기본 `KEY`)가 `migrate` 가능 → 즉시 코드 교체.
- **Forwarder (신규)**: `upgrade_forwarder.sh` 로 새 code_id 를 등록하면 *이후 생성*되는 forwarder 에 적용.
- **Forwarder (기존)**: 각 forwarder 의 admin 은 **factory 컨트랙트**입니다. factory 에 `MigrateForwarder`
  실행 메시지가 없으므로 기존 forwarder 는 스크립트로 일괄 마이그레이션할 수 없습니다.
  일괄 마이그레이션이 필요하면 factory 컨트랙트에 `MigrateForwarder { address, new_code_id, msg }`
  엔드포인트(내부에서 `WasmMsg::Migrate` emit)를 추가해야 합니다. → Check/후속 개선 항목.

## 결정론적 주소 (instantiate2)

`create_forwarder.sh` 는 생성 **전** factory 의 `ForwarderAddress` 쿼리로 주소를 예측·출력하므로,
컨트랙트 생성 전에 해당 주소로 **사전 펀딩**이 가능합니다. 생성 후 예측==실제 주소 일치를 검증합니다.
