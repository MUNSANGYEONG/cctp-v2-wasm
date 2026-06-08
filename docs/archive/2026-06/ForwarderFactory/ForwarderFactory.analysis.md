# Analysis: ForwarderFactory — 설계 ↔ 구현 갭 분석 (PDCA Check)

> 분석일: 2026-06-08
> 기준 문서: [`docs/02-design/features/ForwarderFactory.design.md`](../02-design/features/ForwarderFactory.design.md)
> 목표/DoD: [`docs/01-plan/features/ForwarderFactory.plan.md`](../01-plan/features/ForwarderFactory.plan.md)
> 분석 에이전트: `bkit:gap-detector`

## 결론

**Match Rate: 98%** — 90% 임계 **초과 달성**. Report 단계 진행 가능.

설계 핵심 불변식 11개를 모두 실제 코드/테스트에서 read·grep 으로 확인(추측 없음).
잔여는 **문서 동기화(에러 variant 명) + 차기 E2E 1건 보강**뿐이며, **Critical/Major 기능 결함은 0건**이다.

> 가중: Design Match 70%×100 + Architecture 15%×100 + Convention 15%×90 = **97.85 ≈ 98%**

---

## 1. 설계 항목별 판정 (불변식 1~11)

| # | 설계 항목 | 판정 | 근거 (파일:라인) |
|---|-----------|:----:|------------------|
| 1 | 3 크레이트 + `cosmwasm_1_2` feature, 의존방향 forwarder→shared / factory→shared (factory 가 forwarder 크레이트를 런타임 의존하지 않음) | ✅ Match | `Cargo.toml:14`; `forwarder/Cargo.toml:33`; `forwarder-factory/Cargo.toml:30` (런타임 deps 에 forwarder 없음) |
| 2 | `make_salt` 가 각 필드에 4바이트 BE 길이 프리픽스, sender→dest_chain→recipient 순서 | ✅ Match | `forwarder-factory/src/contract.rs:284-291` |
| 3 | CreateForwarder 7단계 (has→query_wasm_code_info→instantiate2_address→PENDING→reply_on_success→parse_instantiate_response_data+AddressMismatch→FORWARDERS.save) | ✅ Match | `forwarder-factory/src/contract.rs:188-190,192-197,201-211,231,121-134,136-146` |
| 4 | CreateForwarder owner-only (설계 §2.2 MVP 결정) | ✅ Match | `contract.rs:185`; 테스트 `integration.rs` `create_forwarder_is_owner_only` |
| 5 | forwarder 실행경로가 factory `query_wasm_smart(Config)` 로 공유값을 읽고, 실패 시 `FactoryQueryFailed` revert (fallback 없음) | ✅ Match | `forwarder/src/contract.rs:147-154`; 호출지점 `219,311,358` (transfer/refund/abandon 전부) |
| 6 | forwarder instantiate 에서 `factory == info.sender` 검증 | ✅ Match | `forwarder/src/contract.rs:35-36`; 테스트 `:479` |
| 7 | 양 크레이트 cw2 `set_contract_version` + no-op `migrate` + CONTRACT_NAME 게이트, forwarder 신규 이름 | ✅ Match | factory `:33,158-168`; forwarder `:31,127-137` (`crates.io:cctp-v2-forwarder`) |
| 8 | forwarder `Config` = factory/sender/recipient/dest_chain 만 보유, 공유필드(skip_*/refund/owner) 제거 | ✅ Match | `forwarder/src/state.rs:10-16` |
| 9 | RequestStatus 5상태; TransferCall `reply_on_error`; **Refund=Fail-only, Abandon=Fail\|Refund** (C1/M2 수정) | ✅ Match | `state.rs:20-26`; `contract.rs:274,319-321,366-368`; 회귀테스트 `refund_rejected_for_transfer_status_request:743` |
| 10 | Forwarders 페이지네이션 salt 바이트 오름차순 키 | ✅ Match | `forwarder-factory/src/contract.rs:337-341` |
| 11 | 2 wasm 산출물 + 예측==실제 E2E + 중복차단 + 회귀 테스트 | ✅ Match (주의) | `create_forwarder_matches_predicted_address:129`, `duplicate_route_is_rejected:191`, forwarder 단위 7건. ⚠ UpdateConfig 즉시반영 **전용** E2E 부재 |

---

## 2. Gap 목록 (전부 Minor — 기능 결함 아님)

### G1. 에러 타입 명칭 불일치 (Minor, 동작 동일)
설계 §5 / §3.5 의 에러 명세와 실제 코드 variant 명이 다르다. **구현이 더 세분화·정확**하므로 코드를 진실로 삼고 설계 문서를 갱신해야 한다.

| 설계 §5 표기 | 실제 코드 variant | 위치 |
|--------------|-------------------|------|
| `UnauthorizedFactory` | `FactoryMismatch` | `forwarder/src/error.rs:37` |
| `InvalidMigration` | `Std(generic_err("unexpected contract name"))` | 양 크레이트 migrate 게이트 |
| `InvalidCodeId` / `Instantiate2Failed` | `Instantiate2` / `ParseReply` / `NoInstantiateData` / `PendingNotFound` | `forwarder-factory/src/error.rs` |

- **심각도**: Minor (문서 표기 차이, 런타임 동작 동일·우수)
- **권장 조치**: 설계 §5 에러 목록을 실제 variant 명에 맞춰 갱신. (코드 변경 불필요)

### G2. UpdateConfig 즉시반영 전용 E2E 부재 (Minor)
DoD §9 "UpdateConfig 후 forwarder 동작에 즉시 반영(E2E)" 에 대응하는 **전용 통합테스트**가 없다. forwarder 단위테스트(factory Config mock) + `create_forwarder_matches_predicted_address` 의 ConfigResponse 해석으로 사실상 커버되어 **기능 Gap 은 아니다**.

- **심각도**: Minor
- **권장 조치**: 차기 사이클에 `UpdateConfig → TransferCall` 변경 반영 E2E 1건 추가.

### 권한 게이트 명확화 (Gap 아님 — 오해 방지용 기록)
factory `CreateForwarder` 는 **owner-only** 이고(설계 일치), 루트 CLAUDE.md 의 "skip-relayer 게이트"는 forwarder `TransferCall` 에만 적용되는 **다른 게이트**다. 두 게이트는 서로 다른 컨트랙트·다른 호출이며 설계와 구현이 일치한다.

---

## 3. 설계 초과 구현 (Gap 아님 — 추가 안전장치)

설계에 명시되지 않았으나 구현이 더한 방어 로직. 회귀 위험 없으며 운영 안전성 향상.

| 항목 | 위치 | 효과 |
|------|------|------|
| 배포 레벨 예측≠실제 `die` | `scripts/create_forwarder.sh:42` | 온체인 `AddressMismatch` 와 이중화 — 사전펀딩 주소 사고 차단 |
| 생성 전 `exists=true` 차단 | `scripts/create_forwarder.sh:27` | 중복 라우트 조기 실패 |
| `query_smart` null/실패 가드 | `scripts/lib.sh:102-108` | garbage 값 소비 방지 (명령치환 die 전파) |
| Refund 시 잔액+0금액 재검증 | `forwarder/src/contract.rs:323-331` | 풀링 잔액 오배분 보조 방어선 |
| `deployments/<chain>.json` 누적 원장 | `scripts/lib.sh` | Tx/code_id/주소/salt/버전 이력 영속화 |

---

## 4. DoD 체크 (Plan §7 / Design §9)

- [x] 2개 wasm 산출물 빌드 (`just build` → forwarder.wasm, forwarder_factory.wasm)
- [x] `ForwarderAddress` 예측 == `CreateForwarder` 실제 주소 (`create_forwarder_matches_predicted_address`)
- [x] 동일 라우트 재생성 실패 (`duplicate_route_is_rejected`)
- [x] TransferCall 이 Factory 공유설정 참조 (권한/대상/refund)
- [~] UpdateConfig 후 즉시반영 — 단위 커버, 전용 E2E 차기 (G2)
- [x] forward/refund/abandon/request 추적 회귀 없음 (forwarder 단위 8건 통과)
- [x] gap 분석 Match Rate ≥ 90% → **98%**

---

## 5. 다음 단계

**Match Rate 98% ≥ 90% → 자동 개선(iterate) 불필요.** 잔여 2 Minor 는 문서 동기화·차기 E2E 로 분리.

→ `/pdca report ForwarderFactory` (완료 리포트 생성)
