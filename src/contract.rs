use cosmwasm_std::{
    attr, entry_point, to_json_binary, Addr, Coin, Deps, DepsMut, Env, MessageInfo,
    Order, QueryResponse, Reply, Response, StdResult, SubMsg, SubMsgResult, Uint128, WasmMsg,
};
use cw_storage_plus::Bound;

use crate::error::ContractError;
use crate::msg::{
    ConfigResponse, ExecuteMsg, InstantiateMsg, QueryMsg, RequestItem, RequestListResponse,
    RequestOrder, RequestResponse, SkipEntrypointExecuteMsg, SkipTransferCallPayload,
};
use crate::state::{
    Config, RequestStatus, TransferRequest, CONFIG, NEXT_REQUEST_ID, REQUESTS_BY_ID,
    REQUEST_ID_BY_MINT_TX_HASH,
};

const CONTRACT_NAME: &str = "crates.io:cctp-v2-forward-contract";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let owner = match msg.owner {
        Some(owner) => deps.api.addr_validate(&owner)?,
        None => info.sender.clone(),
    };

    let config = Config {
        sender_addr: msg.sender_addr,
        recipient_addr: msg.recipient_addr,
        dest_chain: msg.dest_chain,
        refund_addr: deps.api.addr_validate(&msg.refund_addr)?,
        skip_relayer_addr: deps.api.addr_validate(&msg.skip_relayer_addr)?,
        skip_entrypoint_addr: deps.api.addr_validate(&msg.skip_entrypoint_addr)?,
        owner,
    };

    CONFIG.save(deps.storage, &config)?;
    NEXT_REQUEST_ID.save(deps.storage, &1)?;

    Ok(Response::new().add_attributes([
        attr("action", "instantiate"),
        attr("owner", config.owner),
        attr("sender_addr", config.sender_addr),
        attr("recipient_addr", config.recipient_addr),
        attr("dest_chain", config.dest_chain),
        attr("skip_relayer_addr", config.skip_relayer_addr),
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

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config = CONFIG.load(deps.storage)?;

    Ok(ConfigResponse {
        sender_addr: config.sender_addr,
        recipient_addr: config.recipient_addr,
        dest_chain: config.dest_chain,
        refund_addr: config.refund_addr.to_string(),
        skip_relayer_addr: config.skip_relayer_addr.to_string(),
        skip_entrypoint_addr: config.skip_entrypoint_addr.to_string(),
        owner: config.owner.to_string(),
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
    assert_skip_relayer(&config, &info.sender)?;

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
        contract_addr: config.skip_entrypoint_addr.to_string(),
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

    Ok(Response::new().add_submessage(transfer_submsg).add_attributes([
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
        attr("destination_channel", payload.transfer_call.destination_channel),
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
    assert_owner_or_skip_relayer(&config, &info.sender)?;

    let mut request = load_request_by_mint_tx_hash_mutable(deps.storage, &mint_tx_hash)?;
    if matches!(request.status, RequestStatus::Refund | RequestStatus::Abandon) {
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
        to_address: config.refund_addr.to_string(),
        amount: vec![request.transfer_coin.clone()],
    };

    request.status = RequestStatus::Refund;
    REQUESTS_BY_ID.save(deps.storage, request.id, &request)?;

    Ok(
        Response::new()
            .add_message(refund_msg)
            .add_attributes([
                attr("action", "refund"),
                attr("request_id", request.id.to_string()),
                attr("mint_tx_hash", request.mint_tx_hash.to_string()),
                attr("refund_addr", config.refund_addr.to_string()),
                attr("amount", request.transfer_coin.amount.to_string()),
                attr("denom", request.transfer_coin.denom),
                attr("status", "refund"),
            ]),
    )
}

fn execute_abandon(
    deps: DepsMut,
    info: MessageInfo,
    mint_tx_hash: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    assert_owner_or_skip_relayer(&config, &info.sender)?;

    let mut request = load_request_by_mint_tx_hash_mutable(deps.storage, &mint_tx_hash)?;
    if request.status != RequestStatus::Refund {
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

fn assert_skip_relayer(config: &Config, sender: &Addr) -> Result<(), ContractError> {
    if sender != &config.skip_relayer_addr {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

fn assert_owner_or_skip_relayer(config: &Config, sender: &Addr) -> Result<(), ContractError> {
    if sender != &config.skip_relayer_addr && sender != &config.owner {
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
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{coin, from_json, CosmosMsg, ReplyOn, SubMsgResult};

    use super::*;

    fn instantiate_default(deps: DepsMut) {
        let msg = InstantiateMsg {
            sender_addr: "0xsender".to_string(),
            recipient_addr: "dydx1recipient".to_string(),
            dest_chain: "dydx-mainnet-1".to_string(),
            refund_addr: "inj1refund0000000000000000000000000000000".to_string(),
            skip_relayer_addr: "inj1skip000000000000000000000000000000000".to_string(),
            skip_entrypoint_addr: "inj1entry00000000000000000000000000000000".to_string(),
            owner: Some("inj1owner000000000000000000000000000000000".to_string()),
        };

        let info = mock_info("creator", &[]);
        instantiate(deps, mock_env(), info, msg).expect("instantiate should succeed");
    }

    #[test]
    fn instantiate_and_query_config() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());

        let query_bin = query(deps.as_ref(), mock_env(), QueryMsg::Config {}).expect("query");
        let config: ConfigResponse = from_json(query_bin).expect("decode query response");

        assert_eq!(config.sender_addr, "0xsender");
    }

    #[test]
    fn transfer_call_requires_skip_relayer() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());

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

        deps.querier
            .update_balance("cosmos2contract", vec![coin(100, "uusdc")]);

        let res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("inj1skip000000000000000000000000000000000", &[]),
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
                assert_eq!(contract_addr, "inj1entry00000000000000000000000000000000");
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

        let list = query(
            deps.as_ref(),
            mock_env(),
            QueryMsg::Requests {
                start_after: None,
                limit: Some(10),
                order: Some(RequestOrder::Asc),
            },
        )
        .expect("list query should succeed");
        let list_resp: RequestListResponse = from_json(list).expect("decode list response");
        assert_eq!(list_resp.requests.len(), 1);
        assert_eq!(list_resp.requests[0].mint_tx_hash, "0xmint-2");
    }

    #[test]
    fn reply_marks_request_fail_on_skip_entrypoint_failure() {
        let mut deps = mock_dependencies();
        instantiate_default(deps.as_mut());

        deps.querier
            .update_balance("cosmos2contract", vec![coin(100, "uusdc")]);

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info("inj1skip000000000000000000000000000000000", &[]),
            ExecuteMsg::TransferCall {
                mint_tx_hash: "0xmint-fail".to_string(),
                transfer_coin: coin(30, "uusdc"),
                hook_data: "{\"memo\":\"hook-fail\"}".to_string(),
                destination_channel: "channel-11".to_string(),
            },
        )
        .expect("transfer call should succeed");

        let _res = reply(
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

        deps.querier
            .update_balance("cosmos2contract", vec![coin(100, "uusdc")]);

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info("inj1skip000000000000000000000000000000000", &[]),
            ExecuteMsg::TransferCall {
                mint_tx_hash: "0xmint-refund".to_string(),
                transfer_coin: coin(25, "uusdc"),
                hook_data: "{\"memo\":\"hook-refund\"}".to_string(),
                destination_channel: "channel-12".to_string(),
            },
        )
        .expect("transfer should succeed");

        let refund_res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("inj1owner000000000000000000000000000000000", &[]),
            ExecuteMsg::Refund {
                mint_tx_hash: "0xmint-refund".to_string(),
            },
        )
        .expect("refund should succeed");
        assert_eq!(refund_res.messages.len(), 1);

        execute(
            deps.as_mut(),
            mock_env(),
            mock_info("inj1owner000000000000000000000000000000000", &[]),
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

        deps.querier
            .update_balance("cosmos2contract", vec![coin(80, "uusdc")]);

        let res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("inj1skip000000000000000000000000000000000", &[]),
            ExecuteMsg::TransferCall {
                mint_tx_hash: "0xmint-r-owner".to_string(),
                transfer_coin: coin(10, "uusdc"),
                hook_data: "{\"memo\":\"hook-owner\"}".to_string(),
                destination_channel: "channel-8".to_string(),
            },
        )
        .expect("transfer should succeed");

        let refund_res = execute(
            deps.as_mut(),
            mock_env(),
            mock_info("inj1owner000000000000000000000000000000000", &[]),
            ExecuteMsg::Refund {
                mint_tx_hash: "0xmint-r-owner".to_string(),
            },
        )
        .expect("refund should succeed");

        assert_eq!(res.messages.len(), 1);
        assert_eq!(refund_res.messages.len(), 1);
    }
}
