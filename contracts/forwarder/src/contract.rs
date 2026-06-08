use cosmwasm_std::{
    attr, ensure_eq, entry_point, to_json_binary, Addr, Coin, Deps, DepsMut, Env, MessageInfo,
    Order, QuerierWrapper, QueryResponse, Reply, Response, StdResult, SubMsg, SubMsgResult,
    Uint128, WasmMsg,
};
use cw_storage_plus::Bound;

use forwarder_factory_shared::{FactoryConfigResponse, QueryMsg as FactoryQueryMsg};

use crate::error::ContractError;
use crate::msg::{
    ConfigResponse, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg, RequestItem,
    RequestListResponse, RequestOrder, RequestResponse, SkipEntrypointExecuteMsg,
    SkipTransferCallPayload,
};
use crate::state::{
    Config, RequestStatus, TransferRequest, CONFIG, NEXT_REQUEST_ID, REQUESTS_BY_ID,
    REQUEST_ID_BY_MINT_TX_HASH,
};

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
    NEXT_REQUEST_ID.save(deps.storage, &1)?;

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
            mint_tx_hash,
            transfer_coin,
            hook_data,
            destination_channel,
        } => execute_transfer_call(
            deps,
            env,
            info,
            mint_tx_hash,
            transfer_coin,
            hook_data,
            destination_channel,
        ),
        ExecuteMsg::Refund { mint_tx_hash } => execute_refund(deps, env, info, mint_tx_hash),
        ExecuteMsg::Abandon { mint_tx_hash } => execute_abandon(deps, info, mint_tx_hash),
    }
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<QueryResponse> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&query_config(deps)?),
        QueryMsg::Requests {
            start_after,
            limit,
            order,
        } => to_json_binary(&query_requests(deps, start_after, limit, order)?),
        QueryMsg::RequestByMintTxHash { mint_tx_hash } => {
            to_json_binary(&query_request_by_mint_tx_hash(deps, mint_tx_hash)?)
        }
    }
}

#[entry_point]
pub fn reply(deps: DepsMut, _env: Env, msg: Reply) -> Result<Response, ContractError> {
    match msg.result {
        SubMsgResult::Ok(_) => Ok(Response::new().add_attributes([
            attr("action", "transfer_call_reply"),
            attr("request_id", msg.id.to_string()),
            attr("status", "transfer"),
        ])),
        SubMsgResult::Err(err) => {
            REQUESTS_BY_ID.update(deps.storage, msg.id, |maybe| match maybe {
                Some(mut request) => {
                    request.status = RequestStatus::Fail;
                    Ok(request) // TODO: 에러 메세지 추가
                }
                None => Err(ContractError::RequestNotFound),
            })?;

            Ok(Response::new().add_attributes([
                attr("action", "transfer_call_reply"),
                attr("request_id", msg.id.to_string()),
                attr("status", "fail"),
                attr("error", err),
            ]))
        }
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
        recipient_addr: config.recipient_addr,
        dest_chain: config.dest_chain,
        skip_relayer_addr: fcfg.skip_relayer_addr,
        skip_entrypoint_addr: fcfg.skip_entrypoint_addr,
        refund_addr: config.sender_addr,
        owner: fcfg.owner,
    })
}

fn query_requests(
    deps: Deps,
    start_after: Option<u64>,
    limit: Option<u32>,
    order: Option<RequestOrder>,
) -> StdResult<RequestListResponse> {
    let limit = limit.unwrap_or(20).min(100) as usize;
    let order = order.unwrap_or(RequestOrder::Asc);
    let start = start_after.map(Bound::exclusive);

    let (min, max, range_order) = match order {
        RequestOrder::Asc => (start, None, Order::Ascending),
        RequestOrder::Desc => (None, start, Order::Descending),
    };

    let requests = REQUESTS_BY_ID
        .range(deps.storage, min, max, range_order)
        .take(limit)
        .map(|entry| {
            let (_, req) = entry?;
            Ok(to_request_item(req))
        })
        .collect::<StdResult<Vec<_>>>()?;

    Ok(RequestListResponse { requests })
}

fn query_request_by_mint_tx_hash(deps: Deps, mint_tx_hash: String) -> StdResult<RequestResponse> {
    let request = load_request_by_mint_tx_hash(deps, &mint_tx_hash)?;
    Ok(RequestResponse {
        request: to_request_item(request),
    })
}

#[allow(clippy::too_many_arguments)]
fn execute_transfer_call(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    mint_tx_hash: String,
    transfer_coin: Coin,
    hook_data: String,
    destination_channel: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let fcfg = load_factory_config(deps.querier, &config.factory)?;
    ensure_skip_relayer(&fcfg, &info.sender)?;

    if transfer_coin.amount == Uint128::zero() {
        return Err(ContractError::NoFundsToForward);
    }

    if REQUEST_ID_BY_MINT_TX_HASH.has(deps.storage, mint_tx_hash.as_str()) {
        return Err(ContractError::DuplicateMintTxHash);
    }

    let available = deps
        .querier
        .query_balance(env.contract.address, transfer_coin.denom.clone())?;
    if available.amount < transfer_coin.amount {
        return Err(ContractError::InsufficientRequestFunds);
    }

    // hook_data is provided as JSON.stringify(...) text. Parse and forward as JSON memo.
    let hook_data_json: serde_json::Value =
        serde_json::from_str(&hook_data).map_err(|_| ContractError::MemoEncodeError)?;
    let memo = to_json_binary(&hook_data_json).map_err(|_| ContractError::MemoEncodeError)?;

    // TODO: Skip Entry Point 에 맞도록 payload 수정해야 함.
    let payload = SkipEntrypointExecuteMsg {
        transfer_call: SkipTransferCallPayload {
            destination_channel,
            recipient: config.recipient_addr.clone(),
            destination_chain: config.dest_chain.clone(),
            memo,
        },
    };

    let transfer_msg = WasmMsg::Execute {
        contract_addr: fcfg.skip_entrypoint_addr.clone(),
        msg: to_json_binary(&payload)?,
        funds: vec![transfer_coin.clone()],
    };

    let request_id = NEXT_REQUEST_ID.load(deps.storage)?;
    let mut request = TransferRequest {
        id: request_id,
        mint_tx_hash: mint_tx_hash.clone(),
        transfer_coin,
        source_tx_height: env.block.height,
        source_tx_index: env.transaction.as_ref().map(|tx| tx.index),
        destination_channel: payload.transfer_call.destination_channel.clone(),
        status: RequestStatus::Pending,
        hook_data: Some(hook_data),
    };

    REQUESTS_BY_ID.save(deps.storage, request_id, &request)?;
    REQUEST_ID_BY_MINT_TX_HASH.save(deps.storage, mint_tx_hash.as_str(), &request_id)?;
    NEXT_REQUEST_ID.save(deps.storage, &(request_id + 1))?;

    let transfer_submsg = SubMsg::reply_on_error(transfer_msg, request_id);

    request.status = RequestStatus::Transfer;
    REQUESTS_BY_ID.save(deps.storage, request_id, &request)?;

    Ok(Response::new()
        .add_submessage(transfer_submsg)
        .add_attributes([
            attr("action", "transfer_call"),
            attr("request_id", request_id.to_string()),
            attr("mint_tx_hash", mint_tx_hash),
            attr("source_tx_height", env.block.height.to_string()),
            attr(
                "source_tx_index",
                env.transaction
                    .as_ref()
                    .map(|tx| tx.index)
                    .unwrap_or_default()
                    .to_string(),
            ),
            attr(
                "destination_channel",
                payload.transfer_call.destination_channel,
            ),
            attr("amount", request.transfer_coin.amount.to_string()),
            attr("denom", request.transfer_coin.denom),
            attr("status", "transfer"),
        ]))
}

fn execute_refund(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    mint_tx_hash: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let fcfg = load_factory_config(deps.querier, &config.factory)?;
    ensure_owner_or_skip_relayer(&fcfg, &info.sender)?;

    let mut request = load_request_by_mint_tx_hash_mutable(deps.storage, &mint_tx_hash)?;
    // Refund is only valid for a request whose onward transfer FAILED. A successful
    // (`Transfer`) request has already forwarded its funds out; allowing a refund from
    // `Transfer` would pay out against the pooled balance held for *other* requests
    // (there is no per-request escrow), so the FSM — not the balance check — must gate it.
    if request.status != RequestStatus::Fail {
        return Err(ContractError::InvalidStatusTransition);
    }

    let available = deps
        .querier
        .query_balance(env.contract.address, request.transfer_coin.denom.clone())?;
    if available.amount < request.transfer_coin.amount {
        return Err(ContractError::InsufficientRequestFunds);
    }
    if request.transfer_coin.amount == Uint128::zero() {
        return Err(ContractError::NoFundsToForward);
    }

    let refund_msg = cosmwasm_std::BankMsg::Send {
        to_address: config.sender_addr.clone(),
        amount: vec![request.transfer_coin.clone()],
    };

    request.status = RequestStatus::Refund;
    REQUESTS_BY_ID.save(deps.storage, request.id, &request)?;

    Ok(Response::new().add_message(refund_msg).add_attributes([
        attr("action", "refund"),
        attr("request_id", request.id.to_string()),
        attr("mint_tx_hash", request.mint_tx_hash.to_string()),
        attr("refund_addr", config.sender_addr),
        attr("amount", request.transfer_coin.amount.to_string()),
        attr("denom", request.transfer_coin.denom),
        attr("status", "refund"),
    ]))
}

fn execute_abandon(
    deps: DepsMut,
    info: MessageInfo,
    mint_tx_hash: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    let fcfg = load_factory_config(deps.querier, &config.factory)?;
    ensure_owner_or_skip_relayer(&fcfg, &info.sender)?;

    let mut request = load_request_by_mint_tx_hash_mutable(deps.storage, &mint_tx_hash)?;
    // Abandon is the terminal "give up" state. Reachable directly from `Fail` (operator
    // walks away from a stuck request that cannot be refunded — e.g. insufficient balance)
    // or from `Refund`. Without the `Fail` path a request whose refund cannot complete
    // would be trapped in `Fail` with no terminal state.
    if !matches!(request.status, RequestStatus::Fail | RequestStatus::Refund) {
        return Err(ContractError::InvalidStatusTransition);
    }

    request.status = RequestStatus::Abandon;
    REQUESTS_BY_ID.save(deps.storage, request.id, &request)?;

    Ok(Response::new().add_attributes([
        attr("action", "abandon"),
        attr("request_id", request.id.to_string()),
        attr("mint_tx_hash", request.mint_tx_hash.to_string()),
        attr("status", "abandon"),
    ]))
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

fn to_request_item(request: TransferRequest) -> RequestItem {
    RequestItem {
        id: request.id,
        mint_tx_hash: request.mint_tx_hash,
        transfer_coin: request.transfer_coin,
        source_tx_height: request.source_tx_height,
        source_tx_index: request.source_tx_index,
        destination_channel: request.destination_channel,
        status: request.status,
    }
}

fn load_request_by_mint_tx_hash(deps: Deps, mint_tx_hash: &str) -> StdResult<TransferRequest> {
    let request_id = REQUEST_ID_BY_MINT_TX_HASH.load(deps.storage, mint_tx_hash)?;
    REQUESTS_BY_ID.load(deps.storage, request_id)
}

fn load_request_by_mint_tx_hash_mutable(
    storage: &dyn cosmwasm_std::Storage,
    mint_tx_hash: &str,
) -> Result<TransferRequest, ContractError> {
    let request_id = REQUEST_ID_BY_MINT_TX_HASH
        .load(storage, mint_tx_hash)
        .map_err(|_| ContractError::RequestNotFound)?;
    REQUESTS_BY_ID
        .load(storage, request_id)
        .map_err(|_| ContractError::RequestNotFound)
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info, MockQuerier};
    use cosmwasm_std::{
        coin, from_json, ContractResult, CosmosMsg, OwnedDeps, ReplyOn, SubMsgResult, SystemError,
        SystemResult, WasmQuery,
    };

    use super::*;

    const FACTORY: &str = "factory0000000000000000000000000000000000";
    const RELAYER: &str = "skiprelayer00000000000000000000000000000";
    const ENTRY: &str = "skipentry000000000000000000000000000000";
    const OWNER: &str = "owneraddr000000000000000000000000000000";
    const FORWARDER_CODE_ID: u64 = 7;

    /// Wire the mock querier so `query_wasm_smart(FACTORY, Config {})` returns the
    /// shared config the real factory would serve.
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
            sender_addr: "0xsender".to_string(),
            recipient_addr: "dydx1recipient".to_string(),
            dest_chain: "dydx-mainnet-1".to_string(),
        };
        // Factory is the instantiating sender.
        let info = mock_info(FACTORY, &[]);
        instantiate(deps, mock_env(), info, msg).expect("instantiate should succeed");
    }

    #[test]
    fn instantiate_rejects_factory_mismatch() {
        let mut deps = mock_dependencies();
        let msg = InstantiateMsg {
            factory: FACTORY.to_string(),
            sender_addr: "0xsender".to_string(),
            recipient_addr: "dydx1recipient".to_string(),
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
        let config: ConfigResponse = from_json(query_bin).expect("decode query response");

        assert_eq!(config.factory, FACTORY);
        assert_eq!(config.sender_addr, "0xsender");
        // Resolved from the factory:
        assert_eq!(config.skip_relayer_addr, RELAYER);
        assert_eq!(config.skip_entrypoint_addr, ENTRY);
        assert_eq!(config.refund_addr, "0xsender");
        assert_eq!(config.owner, OWNER);
    }

    #[test]
    fn transfer_call_requires_skip_relayer() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        mock_factory(&mut deps);
        deps.querier
            .update_balance("cosmos2contract", vec![coin(100, "uusdc")]);

        let res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("intruder", &[]),
            ExecuteMsg::TransferCall {
                mint_tx_hash: "0xmint-1".to_string(),
                transfer_coin: coin(100, "uusdc"),
                hook_data: "{\"route\":1}".to_string(),
                destination_channel: "channel-42".to_string(),
            },
        );

        assert_eq!(res, Err(ContractError::Unauthorized));
    }

    #[test]
    fn transfer_call_registers_and_forwards_requested_amount() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        mock_factory(&mut deps);
        deps.querier
            .update_balance("cosmos2contract", vec![coin(100, "uusdc")]);

        let res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(RELAYER, &[]),
            ExecuteMsg::TransferCall {
                mint_tx_hash: "0xmint-2".to_string(),
                transfer_coin: coin(40, "uusdc"),
                hook_data: "{\"memo\":\"hook-data\"}".to_string(),
                destination_channel: "channel-7".to_string(),
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
                assert_eq!(funds.len(), 1);
                assert_eq!(funds[0], coin(40, "uusdc"));
            }
            other => panic!("unexpected message: {other:?}"),
        }
        assert_eq!(res.messages[0].id, 1);
        assert_eq!(res.messages[0].reply_on, ReplyOn::Error);

        let single = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::RequestByMintTxHash {
                mint_tx_hash: "0xmint-2".to_string(),
            },
        )
        .expect("single query should succeed");
        let request: RequestResponse = from_json(single).expect("decode single query response");
        assert_eq!(request.request.status, RequestStatus::Transfer);
    }

    #[test]
    fn reply_marks_request_fail_on_skip_entrypoint_failure() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        mock_factory(&mut deps);
        deps.querier
            .update_balance("cosmos2contract", vec![coin(100, "uusdc")]);

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info(RELAYER, &[]),
            ExecuteMsg::TransferCall {
                mint_tx_hash: "0xmint-fail".to_string(),
                transfer_coin: coin(30, "uusdc"),
                hook_data: "{\"memo\":\"hook-fail\"}".to_string(),
                destination_channel: "channel-11".to_string(),
            },
        )
        .expect("transfer call should succeed");

        reply(
            deps.as_mut(),
            mock_env(),
            Reply {
                id: 1,
                result: SubMsgResult::Err("skip entrypoint execute failed".to_string()),
            },
        )
        .expect("reply should not fail tx and must mark request as fail");

        let single = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::RequestByMintTxHash {
                mint_tx_hash: "0xmint-fail".to_string(),
            },
        )
        .expect("single query should succeed");
        let request: RequestResponse = from_json(single).expect("decode single query response");
        assert_eq!(request.request.status, RequestStatus::Fail);
    }

    #[test]
    fn refund_and_abandon_are_request_scoped() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        mock_factory(&mut deps);
        deps.querier
            .update_balance("cosmos2contract", vec![coin(100, "uusdc")]);

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info(RELAYER, &[]),
            ExecuteMsg::TransferCall {
                mint_tx_hash: "0xmint-refund".to_string(),
                transfer_coin: coin(25, "uusdc"),
                hook_data: "{\"memo\":\"hook-refund\"}".to_string(),
                destination_channel: "channel-12".to_string(),
            },
        )
        .expect("transfer should succeed");

        // Onward transfer fails → request flips to `Fail`, which is the only state
        // a refund is valid from.
        reply(
            deps.as_mut(),
            mock_env(),
            Reply {
                id: 1,
                result: SubMsgResult::Err("skip entrypoint execute failed".to_string()),
            },
        )
        .expect("reply should mark request as fail");

        let refund_res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::Refund {
                mint_tx_hash: "0xmint-refund".to_string(),
            },
        )
        .expect("refund should succeed");
        assert_eq!(refund_res.messages.len(), 1);

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::Abandon {
                mint_tx_hash: "0xmint-refund".to_string(),
            },
        )
        .expect("abandon should succeed after refund");

        let single = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::RequestByMintTxHash {
                mint_tx_hash: "0xmint-refund".to_string(),
            },
        )
        .expect("single query should succeed");
        let request: RequestResponse = from_json(single).expect("decode single query response");
        assert_eq!(request.request.status, RequestStatus::Abandon);
    }

    #[test]
    fn refund_works_for_owner() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        mock_factory(&mut deps);
        deps.querier
            .update_balance("cosmos2contract", vec![coin(80, "uusdc")]);

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info(RELAYER, &[]),
            ExecuteMsg::TransferCall {
                mint_tx_hash: "0xmint-r-owner".to_string(),
                transfer_coin: coin(10, "uusdc"),
                hook_data: "{\"memo\":\"hook-owner\"}".to_string(),
                destination_channel: "channel-8".to_string(),
            },
        )
        .expect("transfer should succeed");

        // Drive the request to `Fail` (the only refundable state).
        reply(
            deps.as_mut(),
            mock_env(),
            Reply {
                id: 1,
                result: SubMsgResult::Err("skip entrypoint execute failed".to_string()),
            },
        )
        .expect("reply should mark request as fail");

        let refund_res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::Refund {
                mint_tx_hash: "0xmint-r-owner".to_string(),
            },
        )
        .expect("refund should succeed");
        assert_eq!(refund_res.messages.len(), 1);
    }

    /// Regression (C1): a successfully-forwarded request stays in `Transfer` and MUST NOT
    /// be refundable — otherwise a refund would pay out against the pooled balance held for
    /// other requests. Also locks the FSM so `Abandon` is reachable directly from `Fail`.
    #[test]
    fn refund_rejected_for_transfer_status_request() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());
        mock_factory(&mut deps);
        deps.querier
            .update_balance("cosmos2contract", vec![coin(100, "uusdc")]);

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info(RELAYER, &[]),
            ExecuteMsg::TransferCall {
                mint_tx_hash: "0xmint-c1".to_string(),
                transfer_coin: coin(25, "uusdc"),
                hook_data: "{\"memo\":\"hook-c1\"}".to_string(),
                destination_channel: "channel-12".to_string(),
            },
        )
        .expect("transfer should succeed");

        // Request is now in `Transfer` (onward submsg dispatched, no error reply). Refund
        // must be rejected even though the contract balance would cover it.
        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::Refund {
                mint_tx_hash: "0xmint-c1".to_string(),
            },
        )
        .expect_err("refund must be rejected for a Transfer-status request");
        assert!(matches!(err, ContractError::InvalidStatusTransition));

        // Abandon must likewise be rejected from `Transfer`.
        let err = execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::Abandon {
                mint_tx_hash: "0xmint-c1".to_string(),
            },
        )
        .expect_err("abandon must be rejected for a Transfer-status request");
        assert!(matches!(err, ContractError::InvalidStatusTransition));

        // After a failure reply the request is `Fail`; Abandon is now allowed directly
        // (M2 — no successful Refund required first).
        reply(
            deps.as_mut(),
            mock_env(),
            Reply {
                id: 1,
                result: SubMsgResult::Err("skip entrypoint execute failed".to_string()),
            },
        )
        .expect("reply should mark request as fail");

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info(OWNER, &[]),
            ExecuteMsg::Abandon {
                mint_tx_hash: "0xmint-c1".to_string(),
            },
        )
        .expect("abandon should succeed directly from Fail");

        let single = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::RequestByMintTxHash {
                mint_tx_hash: "0xmint-c1".to_string(),
            },
        )
        .expect("single query should succeed");
        let request: RequestResponse = from_json(single).expect("decode single query response");
        assert_eq!(request.request.status, RequestStatus::Abandon);
    }
}
