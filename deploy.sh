## 한번에 실행하는 것이 아닌 부분별 복사 붙여넣기 식으로 진행.
## injectived 설치 필요

export WASMD_FILE="$(which injectived)"
export WASM_FILE="/Users/munsangyeong/cosmostation/cctp-v2-forward-contract/artifacts/cctp_v2_forward_contract.wasm"
export CHAIN_ID="injective-888"
export RPC="https://testnet.sentry.tm.injective.network:443"
export WALLET="deploy-test" # inj1xp9cvnaypxnzalcu4kp22xhtmsxexducdx4myx
export TESTER="tester"  # inj1g4d25sx8q7h7y98rpaqcc0w3gkkle9fl24gxpu
export GAS_PRICES="160000000inj"

#####

# $WASMD_FILE keys add "$WALLET" --recover --keyring-backend test ## nemonic 필요. tester 도 같은 방법으로 추가.
$WASMD_FILE keys show "$WALLET" -a --keyring-backend test

#####

$WASMD_FILE tx wasm store "$WASM_FILE" \
  --from "$WALLET" \
  --chain-id "$CHAIN_ID" \
  --node "$RPC" \
  --keyring-backend test \
  --gas auto \
  --gas-adjustment 1.5 \
  --gas-prices $GAS_PRICES \
  --broadcast-mode sync \
  --output json \
  -y | tee /tmp/store_tx.json


export STORE_TX_HASH=$(jq -r '.txhash' /tmp/store_tx.json)
echo "$STORE_TX_HASH"

#####

export CODE_ID=$(
  $WASMD_FILE query tx "$STORE_TX_HASH" --node "$RPC" --output json |
  jq -r '.. | objects | select(.key? == "code_id") | .value' | tail -n1
)
echo "$CODE_ID"

#####

export SENDER="inj1xp9cvnaypxnzalcu4kp22xhtmsxexducdx4myx"
export RECEIPIENT=""

#####
## 아래 부터는 아직 검증되지 않음.
## salt 키를 이용한 initiate2 메세지 형식 테스트 못함.
## skip entry point 빌드 시도 중.

INIT_MSG='{"admin":"inj1xp9cvnaypxnzalcu4kp22xhtmsxexducdx4myx"}'
$WASMD_FILE tx wasm instantiate "$CODE_ID" "$INIT_MSG" \
  --from "$WALLET" \
  --label "token_send" \
  --admin "inj1xp9cvnaypxnzalcu4kp22xhtmsxexducdx4myx" \
  --chain-id "$CHAIN_ID" \
  --node "$RPC" \
  --keyring-backend test \
  --gas auto \
  --gas-adjustment 1.5 \
  --gas-prices "$GAS_PRICES" \
  --broadcast-mode sync \
  --output json \
  -y | tee /tmp/instantiate_tx.json


export INSTANTIATE_TX_HASH=$(jq -r '.txhash' /tmp/instantiate_tx.json)
echo "$INSTANTIATE_TX_HASH"

#####

export CONTRACT_ADDR=$(
  $WASMD_FILE query tx "$INSTANTIATE_TX_HASH" --node "$RPC" --output json |
  jq -r '.. | objects | select(.key? == "_contract_address") | .value' | tail -n1
)
echo "$CONTRACT_ADDR"
export CONTRACT_ADDR="inj1fd2mengvlfmx8ec2hkkehzhje7cpj5a6wmqpmc"

#####