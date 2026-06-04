use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Coin};
use cw_storage_plus::{Item, Map};

#[cw_serde]
pub struct Config {
    pub sender_addr: String,
    pub recipient_addr: String,
    pub dest_chain: String,
    pub refund_addr: Addr,
    pub skip_relayer_addr: Addr,
    pub skip_entrypoint_addr: Addr,
    pub owner: Addr,
}

#[cw_serde]
pub enum RequestStatus {
    Pending,
    Transfer,
    Fail,
    Refund,
    Abandon,
}

#[cw_serde]
pub struct TransferRequest {
    pub id: u64,
    pub mint_tx_hash: String,
    pub transfer_coin: Coin,
    pub source_tx_height: u64,
    pub source_tx_index: Option<u32>,
    pub destination_channel: String,
    pub status: RequestStatus,
    pub hook_data: Option<String>,
}

pub const CONFIG: Item<Config> = Item::new("config");
pub const NEXT_REQUEST_ID: Item<u64> = Item::new("next_request_id");
pub const REQUESTS_BY_ID: Map<u64, TransferRequest> = Map::new("requests_by_id");
pub const REQUEST_ID_BY_MINT_TX_HASH: Map<&str, u64> = Map::new("request_id_by_mint_tx_hash");
