use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Binary, Coin, Uint128};

#[cw_serde]
pub struct MigrateMsg {
}

/// Instantiated by the factory via `instantiate2`. Defined in the shared crate
/// so the factory (producer) and forwarder (consumer) share one JSON schema.
pub use forwarder_factory_shared::ForwarderInstantiateMsg as InstantiateMsg;

#[cw_serde]
pub enum ExecuteMsg {
    TransferCall {
        hook_data: String,
    },
    Refund {},
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},
}

/// Merged view: local route fields + shared config resolved from the factory.
#[cw_serde]
pub struct ConfigResponse {
    pub factory: String,
    pub sender_addr: String,
    pub recipient_addr: String,
    pub dest_chain: String,
    pub owner: String,
}

// TODO: This is a placeholder for the Cw20ReceiveMsg,
// which should be defined in the cw20 crate.
#[cw_serde]
pub struct Cw20ReceiveMsg {
    pub sender: String,
    pub amount: Uint128,
    pub msg: Binary,
}

#[cw_serde]
pub enum Asset {
    Native(Coin),
    Cw20(Addr),
}

#[cw_serde]
pub enum Swap {
    SwapExactAssetIn(SwapExactAssetIn),
    SwapExactAssetOut(SwapExactAssetOut),
}

#[cw_serde]
pub struct SwapExactAssetIn {
    pub swap_venue_name: String,
    pub operations: Vec<SwapOperation>,
}

#[cw_serde]
pub struct SwapExactAssetOut {
    pub swap_venue_name: String,
    pub operations: Vec<SwapOperation>,
}

#[cw_serde]
pub struct SwapOperation {
    pub pool: String,
    pub denom_in: String,
    pub denom_out: String,
}

#[cw_serde]
pub enum Action {
    Transfer(Transfer),
    IbcTransfer(IbcTransfer),
    ContractCall(ContractCall),
}

#[cw_serde]
pub struct Transfer {
    pub to_address: String,
}

#[cw_serde]
pub struct IbcTransfer {
    pub channel: String,
    pub to_address: String,
    pub fee: Option<IbcFee>,
}

#[cw_serde]
pub struct IbcFee {
    pub amount: Vec<Coin>,
    pub gas_limit: Option<String>,
}

#[cw_serde]
pub struct ContractCall {
    pub contract_address: String,
    pub msg: Binary,
}

#[cw_serde]
pub struct Affiliate {
    pub basis_points_fee: Uint128,
    pub address: String,
}

#[cw_serde]
#[allow(clippy::large_enum_variant)]
pub enum SkipEntrypointExecuteMsg {
    Receive(Cw20ReceiveMsg),
    SwapAndActionWithRecover {
        sent_asset: Option<Asset>,
        user_swap: Swap,
        min_asset: Asset,
        timeout_timestamp: u64,
        post_swap_action: Action,
        affiliates: Vec<Affiliate>,
        recovery_addr: Addr,
    },
    SwapAndAction {
        sent_asset: Option<Asset>,
        user_swap: Swap,
        min_asset: Asset,
        timeout_timestamp: u64,
        post_swap_action: Action,
        affiliates: Vec<Affiliate>,
    },
    UserSwap {
        swap: Swap,
        min_asset: Asset,
        remaining_asset: Asset,
        affiliates: Vec<Affiliate>,
    },
    PostSwapAction {
        min_asset: Asset,
        timeout_timestamp: u64,
        post_swap_action: Action,
        exact_out: bool,
    },
    Action {
        sent_asset: Option<Asset>,
        timeout_timestamp: u64,
        action: Action,
        exact_out: bool,
        min_asset: Option<Asset>,
    },
    ActionWithRecover {
        sent_asset: Option<Asset>,
        timeout_timestamp: u64,
        action: Action,
        exact_out: bool,
        min_asset: Option<Asset>,
        recovery_addr: Addr,
    },
}
