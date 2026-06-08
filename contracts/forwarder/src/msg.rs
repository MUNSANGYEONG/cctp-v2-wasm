use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Coin;

use crate::state::RequestStatus;

/// Instantiated by the factory via `instantiate2`. Defined in the shared crate
/// so the factory (producer) and forwarder (consumer) share one JSON schema.
pub use forwarder_factory_shared::ForwarderInstantiateMsg as InstantiateMsg;

#[cw_serde]
pub enum ExecuteMsg {
    TransferCall {
        mint_tx_hash: String,
        transfer_coin: Coin,
        hook_data: String,
        destination_channel: String,
    },
    Refund {
        mint_tx_hash: String,
    },
    Abandon {
        mint_tx_hash: String,
    },
}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(ConfigResponse)]
    Config {},

    #[returns(RequestListResponse)]
    Requests {
        start_after: Option<u64>,
        limit: Option<u32>,
        order: Option<RequestOrder>,
    },

    #[returns(RequestResponse)]
    RequestByMintTxHash { mint_tx_hash: String },
}

/// No-op migrate payload (cw2 versioning hook).
#[cw_serde]
pub struct MigrateMsg {}

#[cw_serde]
pub enum RequestOrder {
    Asc,
    Desc,
}

/// Merged view: local route fields + shared config resolved from the factory.
#[cw_serde]
pub struct ConfigResponse {
    pub factory: String,
    pub sender_addr: String,
    pub recipient_addr: String,
    pub dest_chain: String,
    // Resolved from the parent factory at query time:
    pub skip_relayer_addr: String,
    pub skip_entrypoint_addr: String,
    pub refund_addr: String,
    pub owner: String,
}

#[cw_serde]
pub struct RequestItem {
    pub id: u64,
    pub mint_tx_hash: String,
    pub transfer_coin: Coin,
    pub source_tx_height: u64,
    pub source_tx_index: Option<u32>,
    pub destination_channel: String,
    pub status: RequestStatus,
}

#[cw_serde]
pub struct RequestListResponse {
    pub requests: Vec<RequestItem>,
}

#[cw_serde]
pub struct RequestResponse {
    pub request: RequestItem,
}

// TODO: Skip EntryPoint 에 맞도록 수정 필요
#[cw_serde]
pub struct SkipEntrypointExecuteMsg {
    pub transfer_call: SkipTransferCallPayload,
}

// TODO: Skip EntryPoint 에 맞도록 수정 필요
#[cw_serde]
pub struct SkipTransferCallPayload {
    pub destination_channel: String,
    pub recipient: String,
    pub destination_chain: String,
    pub memo: cosmwasm_std::Binary,
}
