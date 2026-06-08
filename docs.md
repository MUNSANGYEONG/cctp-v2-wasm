# cctp-v2-forward-contract 개발 문서

## 1. 개요

`cctp-v2-forward-contract`는 CCTP(Cross-Chain Transfer Protocol)를 통해 들어온 자산을 IBC(Inter-Blockchain Communication)를 통해 다른 체인으로 전달(forwarding)하는 역할을 수행하는 코즘와즘 스마트 컨트랙트입니다.

이 컨트랙트는 주로 Skip 프로토콜과 연동하여, 특정 주소(`skip_relayer_addr`)로부터의 `TransferCall` 메시지를 받아 IBC 전송을 실행합니다. 전송 실패 시 환불(refund) 및 요청 포기(abandon) 기능을 제공하여 자산을 안전하게 관리합니다.

## 2. 주요 기능

- **자산 포워딩**: `skip_relayer`로부터 `TransferCall` 실행 메시지를 받으면, 지정된 목적지 체인(`dest_chain`)과 수신자(`recipient_addr`)에게 자산을 IBC 전송합니다.
- **전송 상태 관리**: 각 전송 요청은 고유한 ID와 `mint_tx_hash`를 가지며, `Pending`, `Transfer`, `Fail`, `Refund`, `Abandon` 상태로 관리됩니다.
- **환불 (Refund)**: 전송이 실패했거나 특정 조건 하에 컨트랙트의 `admin` 또는 `skip_relayer`가 `Refund`를 실행하면, 초기 설정된 `refund_addr`로 자산을 환불합니다.
- **요청 포기 (Abandon)**: 환불된 요청에 대해 `admin` 또는 `skip_relayer`가 `Abandon`을 실행하여 해당 요청을 최종적으로 종료시킬 수 있습니다.
- **동적 주소 관리**: Skip 프로토콜의 라우터 및 엔트리포인트 주소를 컨트랙트 내에 하드코딩하지 않고, 컨트랙트의 `admin` 주소에 배포된 외부 컨트랙트(`skip-router`)에 질의하여 동적으로 가져옵니다. 이를 통해 유연성과 보안성을 높였습니다.
- **업그레이드 지원**: `migrate` 엔트리 포인트를 구현하여 컨트랙트의 코드를 새로운 버전으로 업그레이드할 수 있습니다.

## 3. 아키텍처 및 주요 로직

### 3.1. 상태 (State) - `src/state.rs`

컨트랙트의 주요 상태 정보는 다음과 같습니다.

- `CONFIG`: `Config` 구조체를 저장하며, 컨트랙트의 기본 설정을 담고 있습니다.
  - `sender_addr`: IBC 전송 시 발신자 주소.
  - `recipient_addr`: 최종 수신자 주소.
  - `dest_chain`: 목적지 체인 이름.
  - `refund_addr`: 환불 시 자산을 받을 주소.
  - `owner`: 컨트랙트의 소유자 주소 (초기 설정 및 권한 확인용).
- `NEXT_REQUEST_ID`: 다음에 생성될 전송 요청의 ID.
- `REQUESTS_BY_ID`: `u64` 타입의 요청 ID를 키로 `TransferRequest` 구조체를 저장하는 맵.
- `REQUEST_ID_BY_MINT_TX_HASH`: `mint_tx_hash`를 키로 요청 ID를 저장하는 맵.

### 3.2. 메시지 (Messages) - `src/msg.rs`

컨트랙트와 상호작용하기 위한 메시지 타입들입니다.

- `InstantiateMsg`: 컨트랙트 초기화 시 필요한 정보를 담습니다.
- `ExecuteMsg`: 컨트랙트의 상태를 변경하는 실행 메시지들입니다.
  - `TransferCall`: 자산 포워딩을 요청합니다. `skip_relayer`만 호출 가능합니다.
  - `Refund`: 특정 전송 요청을 환불 처리합니다. `admin` 또는 `skip_relayer`만 호출 가능합니다.
  - `Abandon`: 환불된 요청을 최종적으로 포기 처리합니다. `admin` 또는 `skip_relayer`만 호출 가능합니다.
- `QueryMsg`: 컨트랙트의 상태를 조회하는 메시지들입니다.
  - `Config`: 컨트랙트의 설정을 조회합니다.
  - `Requests`: 전송 요청 목록을 조회합니다.
  - `RequestByMintTxHash`: `mint_tx_hash`를 이용해 특정 요청을 조회합니다.
- `MigrateMsg`: 컨트랙트 마이그레이션을 위한 빈 메시지입니다.

### 3.3. 핵심 로직 (Contract Logic) - `src/contract.rs`

#### `instantiate`

- 컨트랙트를 초기화하고 `Config`를 설정합니다.
- `owner`가 지정되지 않으면 메시지를 보낸 `info.sender`가 `owner`가 됩니다.
- `cw2::set_contract_version`을 호출하여 컨트랙트 버전 정보를 저장합니다.

#### `execute_transfer_call`

1.  `query_contract_admin`을 호출하여 컨트랙트의 현재 `admin` 주소를 가져옵니다. 이 주소가 `skip-router` 컨트랙트 주소로 사용됩니다.
2.  `query_skip_relayer`를 호출하여 `skip-router` 컨트랙트로부터 `skip_relayer` 주소를 얻어옵니다.
3.  `assert_skip_relayer`를 통해 메시지 발신자가 `skip_relayer`인지 확인합니다.
4.  `mint_tx_hash`의 중복 여부를 확인합니다.
5.  새로운 `TransferRequest`를 생성하고 상태를 `Pending`으로 설정하여 저장합니다.
6.  `query_skip_entrypoint`를 호출하여 `skip-router`로부터 `skip-entrypoint` 주소를 얻어옵니다.
7.  `skip-entrypoint` 컨트랙트의 `Transfer` 함수를 호출하는 `CosmosMsg`를 생성하고, `SubMsg::reply_on_error`를 통해 전송 실패 시 `reply` 함수가 호출되도록 설정합니다.

#### `execute_refund` & `execute_abandon`

1.  `assert_admin_or_skip_relayer`를 호출하여 메시지 발신자가 컨트랙트의 `admin` 또는 `skip_relayer`인지 확인합니다.
2.  `mint_tx_hash`를 이용해 해당 `TransferRequest`를 불러옵니다.
3.  요청의 현재 상태가 유효한지 확인하고, 상태를 각각 `Refund`, `Abandon`으로 변경합니다.
4.  `execute_refund`의 경우, `BankMsg::Send`를 통해 `refund_addr`로 자산을 전송합니다.

#### `reply`

- `execute_transfer_call`에서 보낸 `SubMsg`의 결과에 따라 호출됩니다.
- `SubMsgResult::Err`인 경우, 해당 요청의 상태를 `Fail`로 업데이트하고 에러 메시지를 저장합니다.
- `SubMsgResult::Ok`인 경우, 요청 상태를 `Transfer`로 간주하고 성공 응답을 반환합니다. (현재 코드에서는 `reply_on_error`만 사용하므로 에러 케이스만 처리됩니다.)

#### Helper Functions

- `query_contract_admin`: `WasmQuery::ContractInfo`를 사용하여 체인에 등록된 컨트랙트의 `admin` 주소를 동적으로 조회합니다.
- `query_skip_relayer` / `query_skip_entrypoint`: `admin` 주소(즉, `skip-router` 주소)에 `Addresses {}` 쿼리를 보내 `skip_relayer`와 `skip_entrypoint` 주소를 얻어옵니다.
- `assert_admin_or_skip_relayer` / `assert_skip_relayer`: 권한 확인을 위한 헬퍼 함수입니다.

## 4. 에러 처리 (`src/error.rs`)

`ContractError` 열거형을 통해 발생할 수 있는 다양한 오류 상황을 정의하고 처리합니다.

- `Unauthorized`: 권한 없는 주소의 접근 시 발생합니다.
- `DuplicateMintTxHash`: 중복된 `mint_tx_hash`로 요청 시 발생합니다.
- `RequestNotFound`: 존재하지 않는 요청 조회 시 발생합니다.
- `InvalidStatusTransition`: 유효하지 않은 상태 변경 시도 시 발생합니다. (예: `Pending` 상태에서 `Abandon` 시도)

## 5. 테스트 (`src/contract.rs` 내 `tests` 모듈)

단위 테스트는 `mock_dependencies`와 `MockQuerier`를 사용하여 다양한 시나리오를 검증합니다.

- `setup_skip_router_query`: `MockQuerier`를 설정하여 `WasmQuery::Smart`와 `WasmQuery::ContractInfo`에 대한 모의 응답을 제공합니다. 이를 통해 외부 컨트랙트 의존성을 테스트 환경에서 시뮬레이션합니다.
- `instantiate_and_query_config`: 초기화 및 설정 조회 기능 테스트.
- `transfer_call_requires_skip_relayer`: `skip_relayer`가 아닌 주소의 `TransferCall` 호출이 실패하는지 테스트.
- `transfer_call_registers_and_forwards_requested_amount`: 정상적인 `TransferCall` 흐름 테스트.
- `reply_marks_request_fail_on_skip_entrypoint_failure`: `reply` 로직이 실패 상태를 올바르게 처리하는지 테스트.
- `refund_and_abandon_are_request_scoped`: `Refund` 및 `Abandon` 기능 테스트.

---

이 문서가 `cctp-v2-forward-contract`의 구조와 기능을 이해하는 데 도움이 되기를 바랍니다.
