## 한번에 실행하는 것이 아닌 부분별 복사 붙여넣기 식으로 진행.
## injectived 설치 필요

export WASMD_FILE="$(which injectived)"
export WASM_FILE="/Users/munsangyeong/cosmostation/cctp-v2-forward-contract/artifacts/cctp_v2_forward_contract.wasm"
export CHAIN_ID="injective-888"
export RPC="https://testnet.sentry.tm.injective.network:443"
export WALLET="deploy-test" # inj1xp9cvnaypxnzalcu4kp22xhtmsxexducdx4myx # what
export TESTER="tester"  # inj1g4d25sx8q7h7y98rpaqcc0w3gkkle9fl24gxpu #test
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
## 9930F2977F135331C89355735F51E08ACBCF3DA8BA8BA7B57BD7203A67F30290

#####

export CODE_ID=$(
  $WASMD_FILE query tx "$STORE_TX_HASH" --node "$RPC" --output json |
  jq -r '.. | objects | select(.key? == "code_id") | .value' | tail -n1
)
echo "$CODE_ID"
## 39538
#####

export SENDER="inj1xp9cvnaypxnzalcu4kp22xhtmsxexducdx4myx"
export RECEIPIENT="noble1pnzxqdnmnj0r3gntlpazphl8jc60urj80dwgpw"
export SKIP_RELAYER="inj16urv8a9dprmfthwg5vq6df3mycaq5wkrqxl8mx"
export DEST_CHAIN="grand_1"
export SALT=$(printf '%s|%s|%s|%s' "$SENDER" "$RECEIPIENT" "$DEST_CHAIN" | sha256sum | cut -c1-64)

echo $SALT

## injectived 기반 예상 주소 계산 (환경에 따라 localhost RPC 연결이 필요할 수 있음)
export PREDICTED_CONTRACT_ADDR_INJECTIVED=$(
  $WASMD_FILE query wasm build-address "$CODE_CHECKSUM" "$WALLET_ADDR" "$SALT" --hex 2>/tmp/build_address_err.log || true
)
echo $PREDICTED_CONTRACT_ADDR_INJECTIVED

if [ -n "$PREDICTED_CONTRACT_ADDR_INJECTIVED" ]; then
  echo "Predicted contract address (injectived): $PREDICTED_CONTRACT_ADDR_INJECTIVED"
else
  echo "Predicted contract address (injectived): unavailable"
  echo "Reason: $(tr '\n' ' ' </tmp/build_address_err.log)"
fi

#####
INIT_MSG=$(jq -n \
  --arg sender_addr    "0x304B864fA409a62eFF1cad82A51AEbdC0d933798" \
  --arg recipient_addr "$RECEIPIENT" \
  --arg dest_chain     "$DEST_CHAIN" \
  --arg refund_addr    "$SENDER" \
  --arg skip_relayer   "$SKIP_RELAYER" \
  --arg skip_entry     "$SENDER" \
  --arg owner          "$SENDER" \
  '{
    sender_addr:           $sender_addr,
    recipient_addr:        $recipient_addr,
    dest_chain:            $dest_chain,
    refund_addr:           $refund_addr,
    skip_relayer_addr:     $skip_relayer,
    skip_entrypoint_addr:  $skip_entry,
    owner:                 $owner
  }')

$WASMD_FILE tx wasm instantiate2 "$CODE_ID" "$INIT_MSG" "$SALT" \
  --hex \
  --from "$WALLET" \
  --label "forward_test_contract" \
  --admin "$SENDER" \
  --chain-id "$CHAIN_ID" \
  --node "$RPC" \
  --keyring-backend test \
  --gas auto \
  --gas-adjustment 1.5 \
  --gas-prices "$GAS_PRICES" \
  --broadcast-mode sync \
  --output json \
  -y | tee /tmp/instantiate2_tx.json


export INSTANTIATE_TX_HASH=$(jq -r '.txhash' /tmp/instantiate2_tx.json)
echo "$INSTANTIATE_TX_HASH"
## 184F4295909027327AAC33884EB63290E34D98F4B5C3F9267A48482CF4297BFA

#####

export CONTRACT_ADDR=$(
  $WASMD_FILE query tx "$INSTANTIATE_TX_HASH" --node "$RPC" --output json |
  jq -r '.. | objects | select(.key? == "_contract_address") | .value' | tail -n1
)
echo "$CONTRACT_ADDR"
## inj1kvgh5tn5f772p8x6qzd9vj9xcuz4p3ha28mgsh

## inj1mhqyl33cnel879pldsw84dmfx58xa6fahxusax
#####

# TransferCall — forwarder 컨트랙트에 hook_data를 전달해 Skip EntryPoint로 포워딩
# 호출자: skip relayer 역할의 키 (TESTER)
# forwarder: inj1t9lp8ttwwwnnnmgjjkggxqa9gcfdlwqhjc3c59

export FORWARDER_ADDR="inj1t9lp8ttwwwnnnmgjjkggxqa9gcfdlwqhjc3c59"

# forwarder config 조회
$WASMD_FILE query wasm contract-state smart "$FORWARDER_ADDR" '{"config":{}}' \
  --node "$RPC" \
  --output json | jq .

#####

# hook_data: JSON을 한 줄 문자열로 압축 후 ExecuteMsg 래핑
# 컨트랙트 실제 잔고를 조회해 hook_data의 sent_asset.amount와 min_asset.amount에 반영
# export USDC_DENOM="erc20:0x0C382e685bbeeFE5d3d9C29e29E341fEE8E84C5d"
#
# ACTUAL_BALANCE=$(
#   $WASMD_FILE query bank balance "$FORWARDER_ADDR" "$USDC_DENOM" \
#     --node "$RPC" \
#     --output json | jq -r '.balance.amount // "0"'
# )
# echo "forwarder balance ($USDC_DENOM): $ACTUAL_BALANCE"
#
# # 잔고의 99% 를 전송 금액으로, 98% 를 min_asset으로 사용
# SEND_AMOUNT=$(python3 -c "print(int(int('$ACTUAL_BALANCE') * 99 // 100))")
# MIN_AMOUNT=$(python3  -c "print(int(int('$ACTUAL_BALANCE') * 98 // 100))")
# echo "send_amount: $SEND_AMOUNT  min_amount: $MIN_AMOUNT"
#
# HOOK_DATA=$(jq -c \
#   --arg send "$SEND_AMOUNT" \
#   --arg min  "$MIN_AMOUNT" \
#   '.action_with_recover.sent_asset.native.amount = $send
#    | .action_with_recover.min_asset.native.amount = $min' \
#   /Users/munsangyeong/cosmostation/cctp-v2-forward-contract/contracts/forwarder/test-data/ibc-transfer-hookdata.json)

HOOK_DATA=$(jq -c . /Users/munsangyeong/cosmostation/cctp-v2-forward-contract/contracts/forwarder/test-data/ibc-transfer-hookdata.json)

TRANSFER_CALL_MSG=$(jq -n --arg hook_data "$HOOK_DATA" \
  '{"transfer_call": {"hook_data": $hook_data}}')

$WASMD_FILE tx wasm execute "$FORWARDER_ADDR" "$TRANSFER_CALL_MSG" \
  --from "$WALLET" \
  --chain-id "$CHAIN_ID" \
  --node "$RPC" \
  --keyring-backend test \
  --gas auto \
  --gas-adjustment 1.5 \
  --gas-prices "$GAS_PRICES" \
  --broadcast-mode sync \
  --output json \
  -y | tee /tmp/transfer_call_tx.json

export TRANSFER_CALL_TX_HASH=$(jq -r '.txhash' /tmp/transfer_call_tx.json)
echo "$TRANSFER_CALL_TX_HASH"

#####