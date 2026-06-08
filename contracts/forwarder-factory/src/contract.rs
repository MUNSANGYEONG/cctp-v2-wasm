use cosmwasm_std::{
    attr, ensure_eq, entry_point, instantiate2_address, to_json_binary, Addr, Binary,
    CanonicalAddr, Deps, DepsMut, Env, MessageInfo, Order, QueryResponse, Reply,
    Response, StdError, StdResult, SubMsg, SubMsgResult, WasmMsg,
};
use cw_storage_plus::Bound;
use sha2::{Digest, Sha256};

use forwarder_factory_shared::{
    FactoryConfigResponse, ForwarderAddressResponse, ForwarderInfo, ForwarderInstantiateMsg,
    ForwardersResponse, QueryMsg,
};

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg};
use crate::state::{
    FactoryConfig, ForwarderRecord, PendingForwarder, CONFIG, FORWARDERS, NEXT_REPLY_ID, PENDING,
};

const CONTRACT_NAME: &str = "crates.io:cctp-v2-forwarder-factory";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

const DEFAULT_LIMIT: u32 = 20;
const MAX_LIMIT: u32 = 100;
const PREDICT_CANONICAL_BYTES: usize = 20;

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let owner = match msg.owner {
        Some(o) => deps.api.addr_validate(&o)?,
        None => info.sender.clone(),
    };

    let config = FactoryConfig {
        owner,
        forwarder_code_id: msg.forwarder_code_id,
        skip_relayer_addr: deps.api.addr_validate(&msg.skip_relayer_addr)?,
        skip_entrypoint_addr: deps.api.addr_validate(&msg.skip_entrypoint_addr)?,
    };

    CONFIG.save(deps.storage, &config)?;
    NEXT_REPLY_ID.save(deps.storage, &1)?;

    Ok(Response::new().add_attributes([
        attr("action", "instantiate"),
        attr("owner", config.owner),
        attr("forwarder_code_id", config.forwarder_code_id.to_string()),
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
        ExecuteMsg::CreateForwarder {
            sender_addr,
            dest_chain,
            recipient_addr,
        } => execute_create_forwarder(deps, env, info, sender_addr, dest_chain, recipient_addr),
        ExecuteMsg::UpdateConfig {
            owner,
            forwarder_code_id,
            skip_relayer_addr,
            skip_entrypoint_addr,
        } => execute_update_config(
            deps,
            info,
            owner,
            forwarder_code_id,
            skip_relayer_addr,
            skip_entrypoint_addr,
        ),
    }
}

#[entry_point]
pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<QueryResponse> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&query_config(deps)?),
        QueryMsg::ForwarderAddress {
            sender_addr,
            dest_chain,
            recipient_addr,
        } => to_json_binary(&query_forwarder_address(
            deps,
            env,
            sender_addr,
            dest_chain,
            recipient_addr,
        )?),
        QueryMsg::Forwarders { start_after, limit } => {
            to_json_binary(&query_forwarders(deps, start_after, limit)?)
        }
    }
}

#[entry_point]
pub fn reply(deps: DepsMut, env: Env, msg: Reply) -> Result<Response, ContractError> {
    let pending = PENDING
        .may_load(deps.storage, msg.id)?
        .ok_or(ContractError::PendingNotFound)?;

    let data = match msg.result {
        SubMsgResult::Ok(res) => res.data.ok_or(ContractError::NoInstantiateData)?,
        SubMsgResult::Err(e) => return Err(ContractError::Instantiate2(e)),
    };

    let parsed = cw_utils::parse_instantiate_response_data(data.as_slice())
        .map_err(|e| ContractError::ParseReply(e.to_string()))?;
    let actual = deps.api.addr_validate(&parsed.contract_address)?;

    // Deterministic guarantee: the instantiated address must equal the one we
    // predicted (and may have already pre-funded).
    ensure_eq!(
        actual,
        pending.predicted,
        ContractError::AddressMismatch {
            predicted: pending.predicted.to_string(),
            actual: actual.to_string(),
        }
    );

    FORWARDERS.save(
        deps.storage,
        pending.salt.as_slice(),
        &ForwarderRecord {
            address: actual.clone(),
            sender_addr: pending.sender_addr,
            recipient_addr: pending.recipient_addr,
            dest_chain: pending.dest_chain,
            created_height: env.block.height,
        },
    )?;
    PENDING.remove(deps.storage, msg.id);

    Ok(Response::new().add_attributes([
        attr("action", "create_forwarder_reply"),
        attr("reply_id", msg.id.to_string()),
        attr("forwarder", actual),
        attr("salt", hex::encode(pending.salt.as_slice())),
    ]))
}

#[entry_point]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    let ver = cw2::get_contract_version(deps.storage)?;
    ensure_eq!(
        ver.contract,
        CONTRACT_NAME,
        ContractError::Std(StdError::generic_err(format!(
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

fn execute_create_forwarder(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    sender_addr: String,
    dest_chain: String,
    recipient_addr: String,
) -> Result<Response, ContractError> {
    let config = CONFIG.load(deps.storage)?;
    ensure_eq!(info.sender, config.owner, ContractError::Unauthorized);

    let salt = make_salt(&sender_addr, &dest_chain, &recipient_addr);
    if FORWARDERS.has(deps.storage, salt.as_slice()) {
        return Err(ContractError::ForwarderExists);
    }

    let predicted = predict_address(
        deps.as_ref(),
        &env,
        config.forwarder_code_id,
        salt.as_slice(),
    )?;

    let reply_id = NEXT_REPLY_ID.load(deps.storage)?;
    NEXT_REPLY_ID.save(deps.storage, &(reply_id + 1))?;
    PENDING.save(
        deps.storage,
        reply_id,
        &PendingForwarder {
            salt: salt.clone(),
            sender_addr: sender_addr.clone(),
            recipient_addr: recipient_addr.clone(),
            dest_chain: dest_chain.clone(),
            predicted: predicted.clone(),
        },
    )?;

    let init = ForwarderInstantiateMsg {
        factory: env.contract.address.to_string(),
        sender_addr: sender_addr.clone(),
        recipient_addr: recipient_addr.clone(),
        dest_chain: dest_chain.clone(),
    };

    let wasm = WasmMsg::Instantiate2 {
        // Factory is admin so it can migrate forwarders later.
        admin: Some(env.contract.address.to_string()),
        code_id: config.forwarder_code_id,
        label: format!("forwarder-{}", hex::encode(salt.as_slice())),
        msg: to_json_binary(&init)?,
        funds: vec![],
        salt: salt.clone(),
    };

    Ok(Response::new()
        .add_submessage(SubMsg::reply_on_success(wasm, reply_id))
        .add_attributes([
            attr("action", "create_forwarder"),
            attr("reply_id", reply_id.to_string()),
            attr("predicted", predicted),
            attr("sender_addr", sender_addr),
            attr("dest_chain", dest_chain),
            attr("recipient_addr", recipient_addr),
            attr("salt", hex::encode(salt.as_slice())),
        ]))
}

#[allow(clippy::too_many_arguments)]
fn execute_update_config(
    deps: DepsMut,
    info: MessageInfo,
    owner: Option<String>,
    forwarder_code_id: Option<u64>,
    skip_relayer_addr: Option<String>,
    skip_entrypoint_addr: Option<String>,
) -> Result<Response, ContractError> {
    let mut config = CONFIG.load(deps.storage)?;
    ensure_eq!(info.sender, config.owner, ContractError::Unauthorized);

    if let Some(o) = owner {
        config.owner = deps.api.addr_validate(&o)?;
    }
    if let Some(code_id) = forwarder_code_id {
        config.forwarder_code_id = code_id;
    }
    if let Some(r) = skip_relayer_addr {
        config.skip_relayer_addr = deps.api.addr_validate(&r)?;
    }
    if let Some(e) = skip_entrypoint_addr {
        config.skip_entrypoint_addr = deps.api.addr_validate(&e)?;
    }
    CONFIG.save(deps.storage, &config)?;

    Ok(Response::new().add_attributes([
        attr("action", "update_config"),
        attr("owner", config.owner),
        attr("forwarder_code_id", config.forwarder_code_id.to_string()),
    ]))
}

/// salt = sha256( len(sender)||sender || len(dest)||dest || len(recip)||recip ).
/// The 4-byte big-endian length prefix per field prevents concatenation
/// collisions (e.g. ("ab","c") vs ("a","bc")).
fn make_salt(sender: &str, dest_chain: &str, recipient: &str) -> Binary {
    let mut hasher = Sha256::new();
    for part in [sender, dest_chain, recipient] {
        hasher.update((part.len() as u32).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    Binary::from(hasher.finalize().to_vec())
}

fn shorten_canonical_address(canonical: &CanonicalAddr, target_len: usize) -> StdResult<CanonicalAddr> {
    let bytes = canonical.0.as_slice();
    if bytes.len() < target_len {
        return Err(StdError::generic_err(format!(
            "canonical address length {} is shorter than expected {}",
            bytes.len(),
            target_len,
        )));
    }
    Ok(CanonicalAddr(Binary::from(bytes[..target_len].to_vec())))
}

/// Predict the instantiate2 address for `salt` under the forwarder code_id.
fn predict_address(deps: Deps, env: &Env, code_id: u64, salt: &[u8]) -> StdResult<Addr> {
    let code_info = deps.querier.query_wasm_code_info(code_id)?;
    let creator = deps.api.addr_canonicalize(env.contract.address.as_str())?;
    let canonical = instantiate2_address(code_info.checksum.as_slice(), &creator, salt)
        .map_err(|e| StdError::generic_err(format!("instantiate2_address: {e}")))?;
    let truncated = shorten_canonical_address(&canonical, PREDICT_CANONICAL_BYTES)?;
    deps.api.addr_humanize(&truncated)
}

fn query_config(deps: Deps) -> StdResult<FactoryConfigResponse> {
    let config = CONFIG.load(deps.storage)?;
    Ok(FactoryConfigResponse {
        owner: config.owner.to_string(),
        forwarder_code_id: config.forwarder_code_id,
        skip_relayer_addr: config.skip_relayer_addr.to_string(),
        skip_entrypoint_addr: config.skip_entrypoint_addr.to_string(),
    })
}

fn query_forwarder_address(
    deps: Deps,
    env: Env,
    sender_addr: String,
    dest_chain: String,
    recipient_addr: String,
) -> StdResult<ForwarderAddressResponse> {
    let config = CONFIG.load(deps.storage)?;
    let salt = make_salt(&sender_addr, &dest_chain, &recipient_addr);
    let address = predict_address(deps, &env, config.forwarder_code_id, salt.as_slice())?;
    let exists = FORWARDERS.has(deps.storage, salt.as_slice());
    Ok(ForwarderAddressResponse {
        address: address.to_string(),
        exists,
    })
}

fn query_forwarders(
    deps: Deps,
    start_after: Option<String>,
    limit: Option<u32>,
) -> StdResult<ForwardersResponse> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    // start_after is the hex-encoded salt of the last item from the prior page.
    let start_vec = start_after.as_deref().and_then(|s| hex::decode(s).ok());
    let start = start_vec.as_deref().map(Bound::exclusive);

    let forwarders = FORWARDERS
        .range(deps.storage, start, None, Order::Ascending)
        .take(limit)
        .map(|item| {
            let (_salt, rec) = item?;
            Ok(ForwarderInfo {
                address: rec.address.to_string(),
                sender_addr: rec.sender_addr,
                recipient_addr: rec.recipient_addr,
                dest_chain: rec.dest_chain,
                created_height: rec.created_height,
            })
        })
        .collect::<StdResult<Vec<_>>>()?;

    Ok(ForwardersResponse { forwarders })
}
