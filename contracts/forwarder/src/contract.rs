use cosmwasm_std::{
    attr, ensure_eq, entry_point, to_json_binary, Addr, Binary, Coin, Deps, DepsMut, Env,
    MessageInfo, QuerierWrapper, QueryResponse, Response, StdResult, Uint128, WasmMsg,
};

use forwarder_factory_shared::{FactoryConfigResponse, QueryMsg as FactoryQueryMsg};

use crate::error::ContractError;
use crate::msg::{
    ConfigResponse, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg,
};
use crate::state::{Config, CONFIG};

const CONTRACT_NAME: &str = "crates.io:cctp-v2-forwarder";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    // The factory instantiates the forwarder (via instantiate2), so the declared
    // factory must equal the sender. This binds the forwarder to its config source.
    let factory = deps.api.addr_validate(&msg.factory)?;
    ensure_eq!(factory, info.sender, ContractError::FactoryMismatch);

    let config = Config {
        factory,
        sender_addr: msg.sender_addr,
        recipient_addr: msg.recipient_addr,
        dest_chain: msg.dest_chain,
    };

    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attributes([
        attr("action", "instantiate"),
        attr("factory", config.factory),
        attr("sender_addr", config.sender_addr),
        attr("recipient_addr", config.recipient_addr),
        attr("dest_chain", config.dest_chain),
    ]))
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::TransferCall {
            hook_data,
        } => execute_transfer_call(
            deps,
            env,
            info,
            hook_data,
        ),
        ExecuteMsg::Refund {} => execute_refund(deps, env, info),
    }
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<QueryResponse> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&query_config(deps)?),
    }
}

#[entry_point]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    let ver = cw2::get_contract_version(deps.storage)?;
    ensure_eq!(
        ver.contract,
        CONTRACT_NAME,
        ContractError::Std(cosmwasm_std::StdError::generic_err(format!(
            "unexpected contract name: {}",
            ver.contract
        )))
    );
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new().add_attributes([
        attr("action", "migrate"),
        attr("from_version", ver.version),
        attr("to_version", CONTRACT_VERSION),
    ]))
}

/// Read the shared config from the parent factory. Any failure reverts the tx —
/// the forwarder never acts on missing/stale routing parameters.
fn load_factory_config(
    querier: QuerierWrapper,
    factory: &Addr,
) -> Result<FactoryConfigResponse, ContractError> {
    querier
        .query_wasm_smart(factory.to_string(), &FactoryQueryMsg::Config {})
        .map_err(|e| ContractError::FactoryQueryFailed(e.to_string()))
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    let fcfg: FactoryConfigResponse = deps
        .querier
        .query_wasm_smart(config.factory.to_string(), &FactoryQueryMsg::Config {})?;

    Ok(ConfigResponse {
        factory: config.factory.to_string(),
        sender_addr: config.sender_addr.clone(),
        recipient_addr: config.recipient_addr.clone(),
        dest_chain: config.dest_chain.clone(),
        owner: fcfg.owner,
    })
}

fn execute_transfer_call(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    hook_data: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let fcfg = load_factory_config(deps.querier, &config.factory)?;
    ensure_skip_relayer(&fcfg, &info.sender)?;

    // hook_data가 0x hex 인코딩된 경우 UTF-8 문자열로 디코딩한다.
    let hook_data = if hook_data.starts_with("0x") || hook_data.starts_with("0X") {
        let bytes = hex::decode(&hook_data[2..]).map_err(|_| ContractError::MemoEncodeError)?;
        String::from_utf8(bytes).map_err(|_| ContractError::MemoEncodeError)?
    } else {
        hook_data
    };

    // hook_data JSON을 파싱해 sent_asset에서 전송할 코인을 추출한다.
    let hook_json: serde_json::Value =
        serde_json::from_str(&hook_data).map_err(|_| ContractError::MemoEncodeError)?;
    let transfer_coin = extract_sent_asset(&hook_json)?;

    if transfer_coin.amount == Uint128::zero() {
        return Err(ContractError::NoFundsToForward);
    }

    // 컨트랙트가 충분한 잔고를 보유하고 있는지 확인한다.
    let available = deps
        .querier
        .query_balance(env.contract.address.clone(), transfer_coin.denom.clone())?;
    if available.amount < transfer_coin.amount {
        return Err(ContractError::InsufficientRequestFunds);
    }

    // Pass hook_data bytes directly as the EntryPoint execute message
    let transfer_msg = WasmMsg::Execute {
        contract_addr: fcfg.skip_entrypoint_addr.clone(),
        msg: Binary::from(hook_data.as_bytes()),
        funds: vec![transfer_coin],
    };

    Ok(Response::new()
        .add_message(transfer_msg)
        .add_attributes([
            attr("action", "transfer_call"),
            attr("source_tx_height", env.block.height.to_string()),
            attr(
                "source_tx_index",
                env.transaction
                    .as_ref()
                    .map(|tx| tx.index)
                    .unwrap_or_default()
                    .to_string(),
            ),
        ]))
}

/// hook_data JSON에서 `sent_asset` 코인을 추출한다.
/// 형식: `<top_level_key>.sent_asset.<asset_type>.{denom, amount}`
/// asset_type 키(예: "native", "cw20")는 가변이므로 첫 번째 키를 사용한다.
/// top_level_key는 `action` 또는 `action_with_recover` 만 허용한다.
fn extract_sent_asset(hook_json: &serde_json::Value) -> Result<Coin, ContractError> {
    let obj = hook_json
        .as_object()
        .ok_or(ContractError::InvalidHookData)?;

    // action / action_with_recover 만 허용 — swap 계열은 지원하지 않는다.
    let allowed_keys = ["action", "action_with_recover"];

    let (_, inner) = obj
        .iter()
        .find(|(k, _)| allowed_keys.contains(&k.as_str()))
        .ok_or(ContractError::InvalidHookData)?;

    let sent_asset = inner
        .get("sent_asset")
        .and_then(|v| v.as_object())
        .ok_or(ContractError::InvalidHookData)?;

    // asset_type 키(native, cw20 등)는 고정이 아니므로 첫 번째 항목을 사용한다.
    let (_, asset_value) = sent_asset
        .iter()
        .next()
        .ok_or(ContractError::InvalidHookData)?;

    let denom = asset_value
        .get("denom")
        .and_then(|v| v.as_str())
        .ok_or(ContractError::InvalidHookData)?
        .to_string();

    let amount_str = asset_value
        .get("amount")
        .and_then(|v| v.as_str())
        .ok_or(ContractError::InvalidHookData)?;

    let amount: Uint128 = amount_str
        .parse()
        .map_err(|_| ContractError::InvalidHookData)?;

    Ok(Coin { denom, amount })
}

fn execute_refund(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let fcfg = load_factory_config(deps.querier, &config.factory)?;
    ensure_owner_or_skip_relayer(&fcfg, &info.sender)?;

    // Send ALL contract balances to config.sender_addr (the originating Injective address).
    // If sender_addr is a 0x EVM address, convert it to inj1 bech32 format first.
    let refund_addr = to_injective_addr(&config.sender_addr)?;

    let balances = deps.querier.query_all_balances(env.contract.address)?;
    if balances.is_empty() {
        return Err(ContractError::NoFundsToForward);
    }

    let refund_msg = cosmwasm_std::BankMsg::Send {
        to_address: refund_addr.clone(),
        amount: balances.clone(),
    };

    Ok(Response::new().add_message(refund_msg).add_attributes([
        attr("action", "refund"),
        attr("refund_addr", refund_addr),
        attr(
            "amounts",
            balances
                .iter()
                .map(|c| format!("{}{}", c.amount, c.denom))
                .collect::<Vec<_>>()
                .join(","),
        ),
    ]))
}

/// Convert an arbitrary address string to an `inj1` bech32 address.
///
/// - Already an `inj1` address → returned as-is.
/// - `0x`-prefixed 20-byte hex (EVM address) → decoded and re-encoded with `inj` prefix.
/// - Any other bech32 string → hrp stripped, raw bytes re-encoded with `inj` prefix.
fn to_injective_addr(addr: &str) -> Result<String, ContractError> {
    use bech32::{FromBase32, ToBase32, Variant};

    if addr.starts_with("inj1") {
        return Ok(addr.to_string());
    }

    let bytes: Vec<u8> = if addr.starts_with("0x") || addr.starts_with("0X") {
        hex::decode(&addr[2..]).map_err(|_| ContractError::InvalidRefundAddress)?
    } else {
        // Assume another bech32 format — strip the HRP and re-use the raw bytes
        let (_, data, _) = bech32::decode(addr).map_err(|_| ContractError::InvalidRefundAddress)?;
        Vec::<u8>::from_base32(&data).map_err(|_| ContractError::InvalidRefundAddress)?
    };

    if bytes.len() != 20 {
        return Err(ContractError::InvalidRefundAddress);
    }

    bech32::encode("inj", bytes.to_base32(), Variant::Bech32)
        .map_err(|_| ContractError::InvalidRefundAddress)
}

fn ensure_skip_relayer(fcfg: &FactoryConfigResponse, sender: &Addr) -> Result<(), ContractError> {
    if sender.as_str() != fcfg.skip_relayer_addr {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

fn ensure_owner_or_skip_relayer(
    fcfg: &FactoryConfigResponse,
    sender: &Addr,
) -> Result<(), ContractError> {
    if sender.as_str() != fcfg.skip_relayer_addr && sender.as_str() != fcfg.owner {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info, MockQuerier};
    use cosmwasm_std::{
        coin, coins, from_json, BankMsg, ContractResult, CosmosMsg, OwnedDeps, SystemError,
        SystemResult, WasmQuery,
    };

    use super::*;

    const FACTORY: &str = "factory0000000000000000000000000000000000";
    const RELAYER: &str = "inj1g4d25sx8q7h7y98rpaqcc0w3gkkle9fl24gxpu";
    const ENTRY: &str = "inj1uj67l73rn8ffzxryzf062ackzpfjfk0qnh7080";
    const OWNER: &str = "inj1g4d25sx8q7h7y98rpaqcc0w3gkkle9fl24gxpu";
    const RECIPIENT: &str = "dydx1recipient";
    // 20-byte EVM address — will be converted to inj1 bech32 on refund
    const SENDER: &str = "0x455AAA40C707AFE214E30f418C3DD145aDFC953F";

    const FORWARDER_CODE_ID: u64 = 7;

    fn mock_factory(
        deps: &mut OwnedDeps<impl cosmwasm_std::Storage, impl cosmwasm_std::Api, MockQuerier>,
    ) {
        deps.querier.update_wasm(|query| match query {
            WasmQuery::Smart { contract_addr, .. } if contract_addr == FACTORY => {
                let resp = FactoryConfigResponse {
                    owner: OWNER.to_string(),
                    forwarder_code_id: FORWARDER_CODE_ID,
                    skip_relayer_addr: RELAYER.to_string(),
                    skip_entrypoint_addr: ENTRY.to_string(),
                };
                SystemResult::Ok(ContractResult::Ok(to_json_binary(&resp).unwrap()))
            }
            _ => SystemResult::Err(SystemError::UnsupportedRequest {
                kind: "only factory config query is mocked".to_string(),
            }),
        });
    }

    fn instantiate_default(deps: DepsMut) {
        let msg = InstantiateMsg {
            factory: FACTORY.to_string(),
            sender_addr: SENDER.to_string(),
            recipient_addr: RECIPIENT.to_string(),
            dest_chain: "dydx-mainnet-1".to_string(),
        };
        let info = mock_info(FACTORY, &[]);
        instantiate(deps, mock_env(), info, msg).expect("instantiate should succeed");
    }


    #[test]
    fn instantiate_rejects_factory_mismatch() {
        let mut deps = mock_dependencies();
        let msg = InstantiateMsg {
            factory: FACTORY.to_string(),
            sender_addr: SENDER.to_string(),
            recipient_addr: RECIPIENT.to_string(),
            dest_chain: "dydx-mainnet-1".to_string(),
        };
        let res = instantiate(
            deps.as_mut(),
            mock_env(),
            mock_info("not-factory", &[]),
            msg,
        );
        assert_eq!(res, Err(ContractError::FactoryMismatch));
    }

    #[test]
    fn instantiate_and_query_config() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        mock_factory(&mut deps);

        let query_bin = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).expect("query");
        let config: ConfigResponse = from_json(query_bin).expect("decode");

        assert_eq!(config.factory, FACTORY);
        assert_eq!(config.sender_addr, SENDER);
        assert_eq!(config.owner, OWNER);
    }

    #[test]
    fn transfer_call_requires_skip_relayer() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        mock_factory(&mut deps);

        let res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("intruder", &[coin(100, "uusdc")]),
            ExecuteMsg::TransferCall {
                hook_data: "{}".to_string(),
            },
        );
        assert_eq!(res, Err(ContractError::Unauthorized));
    }

    #[test]
    fn transfer_call_forwards_to_entrypoint() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        mock_factory(&mut deps);
        // 컨트랙트 잔고 설정 (브릿지된 USDC)
        deps.querier
            .update_balance("cosmos2contract", vec![coin(100, "uusdc")]);

        // sent_asset.native 에 denom/amount 포함된 hook_data
        let hook_data = r#"{"action_with_recover":{"sent_asset":{"native":{"denom":"uusdc","amount":"40"}},"timeout_timestamp":999999999999999999,"action":{"ibc_transfer":{"ibc_info":{"source_channel":"channel-0","receiver":"dydx1recipient","fee":null,"memo":"","recover_address":"cosmos1recover","encoding":null,"eureka_fee":null},"fee_swap":null}},"exact_out":false,"min_asset":{"native":{"denom":"uusdc","amount":"39"}},"recovery_addr":"cosmos1recover"}}"#;

        let res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(RELAYER, &[]),
            ExecuteMsg::TransferCall {
                hook_data: hook_data.to_string(),
            },
        )
        .expect("transfer_call should succeed");

        assert_eq!(res.messages.len(), 1);
        match &res.messages[0].msg {
            CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr,
                funds,
                ..
            }) => {
                assert_eq!(contract_addr, ENTRY);
                assert_eq!(funds[0], coin(40, "uusdc"));
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn refund_sends_all_balances_to_sender() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        mock_factory(&mut deps);
        // Contract holds multiple denoms
        deps.querier.update_balance(
            "cosmos2contract",
            vec![coin(500, "uusdc"), coin(100, "uatom")],
        );

        let res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::Refund {},
        )
        .expect("refund should succeed");

        assert_eq!(res.messages.len(), 1);
        match &res.messages[0].msg {
            CosmosMsg::Bank(BankMsg::Send { to_address, amount }) => {
                let expected = to_injective_addr(SENDER).expect("should convert SENDER to inj1");
                assert_eq!(to_address, &expected);
                assert!(amount.contains(&coin(500, "uusdc")));
                assert!(amount.contains(&coin(100, "uatom")));
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn refund_requires_owner_or_relayer() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        mock_factory(&mut deps);
        deps.querier
            .update_balance("cosmos2contract", coins(100, "uusdc"));

        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("intruder", &[]),
            ExecuteMsg::Refund {},
        )
        .expect_err("should reject unauthorized caller");
        assert!(matches!(err, ContractError::Unauthorized));
    }

    #[test]
    fn refund_fails_when_no_balance() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        mock_factory(&mut deps);
        // No balance on contract

        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::Refund {},
        )
        .expect_err("should fail with no funds");
        assert!(matches!(err, ContractError::NoFundsToForward));
    }
}
