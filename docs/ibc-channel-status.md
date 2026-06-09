# IBC 채널 상태 (injective-888 테스트넷)

조회 일자: 2026-06-09  
네트워크: **injective-888** (Injective 테스트넷)  
REST 엔드포인트: `https://testnet.sentry.lcd.injective.network`

## 채널 현황

| channel     | port     | state          | ordering  | 상대 chain_id     | 상대 channel | connection     |
| ----------- | -------- | -------------- | --------- | ----------------- | ------------ | -------------- |
| channel-23  | transfer | **STATE_OPEN** | UNORDERED | theta-testnet-001 | channel-686  | connection-27  |
| channel-40  | transfer | **STATE_OPEN** | UNORDERED | theta-testnet-001 | channel-1579 | connection-68  |
| channel-68  | transfer | **STATE_OPEN** | UNORDERED | theta-testnet-001 | channel-2707 | connection-93  |
| channel-160 | transfer | **STATE_OPEN** | UNORDERED | theta-testnet-001 | channel-3440 | connection-184 |
| channel-189 | transfer | **STATE_OPEN** | UNORDERED | theta-testnet-001 | channel-3602 | connection-197 |

> 5개 채널 모두 **STATE_OPEN** 이며, 모두 Cosmos Hub 테스트넷(`theta-testnet-001`)과 연결되어 있습니다.

---

## 채널 상태 확인 방법

### 1. REST API (가장 간단)

```bash
# 단일 채널 조회
EP="https://testnet.sentry.lcd.injective.network"
curl -s "${EP}/ibc/core/channel/v1/channels/<channel-id>/ports/transfer" | jq .

# 예시: channel-23
curl -s "${EP}/ibc/core/channel/v1/channels/channel-23/ports/transfer" | jq '{
  state: .channel.state,
  counterparty: .channel.counterparty,
  connection: .channel.connection_hops
}'
```

응답에서 확인할 필드:

| 필드                              | 의미                                                                            |
| --------------------------------- | ------------------------------------------------------------------------------- |
| `channel.state`                   | `STATE_OPEN` = 정상, `STATE_CLOSED` = 닫힘, `STATE_INIT/TRYOPEN` = 개설 진행 중 |
| `channel.counterparty.channel_id` | 상대 체인의 채널 ID                                                             |
| `channel.counterparty.port_id`    | 상대 체인의 포트 (ICS-20 전송이면 `transfer`)                                   |
| `channel.connection_hops`         | 이 채널이 사용하는 connection ID                                                |
| `channel.ordering`                | `ORDER_UNORDERED` (ICS-20 표준)                                                 |

---

### 2. CLI (injectived 바이너리)

```bash
# 단일 채널 조회
injectived query ibc channel end transfer channel-23 \
  --node https://testnet.sentry.tm.injective.network:443

# 전체 채널 목록 (페이지네이션)
injectived query ibc channel channels \
  --node https://testnet.sentry.tm.injective.network:443 \
  --limit 50 --page 1

# transfer 포트만 필터
injectived query ibc channel channels \
  --node https://testnet.sentry.tm.injective.network:443 \
  -o json | jq '[.channels[] | select(.port_id=="transfer")]'
```

---

### 3. 상대 체인 확인 (connection → client_state)

채널에서 `connection_hops`를 구한 뒤 해당 connection의 client_state를 조회하면  
상대 체인의 `chain_id`를 알 수 있습니다.

```bash
EP="https://testnet.sentry.lcd.injective.network"
CONN="connection-27"  # channel-23 의 connection

curl -s "${EP}/ibc/core/connection/v1/connections/${CONN}/client_state" \
  | jq '{chain_id: .identified_client_state.client_state.chain_id}'
```

---

### 4. 여러 채널 일괄 확인 (스크립트)

```bash
#!/usr/bin/env bash
EP="https://testnet.sentry.lcd.injective.network"
CHANNELS=(channel-23 channel-40 channel-68 channel-160 channel-189)

printf "%-14s %-12s %-20s %-14s\n" "channel" "state" "counterparty_chain" "cp_channel"
printf '%s\n' "----------------------------------------------------------------------"

for ch in "${CHANNELS[@]}"; do
  result=$(curl -s "${EP}/ibc/core/channel/v1/channels/${ch}/ports/transfer")

  # 에러 처리
  if echo "${result}" | python3 -c "import sys,json; d=json.load(sys.stdin); exit(0 if 'code' not in d else 1)" 2>/dev/null; then
    state=$(echo "${result}" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['channel']['state'].replace('STATE_',''))")
    cp_ch=$(echo "${result}" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['channel']['counterparty']['channel_id'])")
    conn=$(echo "${result}" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['channel']['connection_hops'][0])")

    chain=$(curl -s "${EP}/ibc/core/connection/v1/connections/${conn}/client_state" \
      | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['identified_client_state']['client_state']['chain_id'])")

    printf "%-14s %-12s %-20s %-14s\n" "${ch}" "${state}" "${chain}" "${cp_ch}"
  else
    printf "%-14s %-12s\n" "${ch}" "NOT_FOUND"
  fi
done
```

---

### 5. 패킷 미전달(stuck) 확인

채널이 OPEN이어도 릴레이어가 패킷을 전달하지 못한 경우가 있습니다.

```bash
EP="https://testnet.sentry.lcd.injective.network"

# 미전달 패킷 조회
curl -s "${EP}/ibc/core/channel/v1/channels/channel-23/ports/transfer/packet_commitments" \
  | jq '{pending_count: (.commitments | length), sequences: [.commitments[].sequence]}'

# 미수신 확인응답(ack) 조회
curl -s "${EP}/ibc/core/channel/v1/channels/channel-23/ports/transfer/packet_acknowledgements" \
  | jq '{ack_count: (.acknowledgements | length)}'
```

---

### 주요 엔드포인트

| 네트워크                 | REST                                           | RPC (Tendermint)                                  |
| ------------------------ | ---------------------------------------------- | ------------------------------------------------- |
| injective-888 (테스트넷) | `https://testnet.sentry.lcd.injective.network` | `https://testnet.sentry.tm.injective.network:443` |
| injective-1 (메인넷)     | `https://lcd.injective.network`                | `https://tm.injective.network`                    |
