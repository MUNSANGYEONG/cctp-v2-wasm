use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Binary};
use cw_storage_plus::{Item, Map};

/// Shared config applied to every forwarder this factory creates. A single
/// update here changes behavior for all forwarders (they read it at runtime).
#[cw_serde]
pub struct FactoryConfig {
    pub owner: Addr,
    pub forwarder_code_id: u64,
    pub skip_relayer_addr: Addr,
    pub skip_entrypoint_addr: Addr,
}

/// Registry entry for an instantiated forwarder. Route fields are kept in
/// plaintext (the salt is a one-way hash of them).
#[cw_serde]
pub struct ForwarderRecord {
    pub address: Addr,
    pub sender_addr: String,
    pub recipient_addr: String,
    pub dest_chain: String,
    pub created_height: u64,
}

/// In-flight forwarder creation, awaiting the instantiate2 reply.
#[cw_serde]
pub struct PendingForwarder {
    pub salt: Binary,
    pub sender_addr: String,
    pub recipient_addr: String,
    pub dest_chain: String,
    pub predicted: Addr,
}

pub const CONFIG: Item<FactoryConfig> = Item::new("factory_config");
/// Keyed by the raw salt bytes (= sha256 of length-prefixed route fields).
pub const FORWARDERS: Map<&[u8], ForwarderRecord> = Map::new("forwarders");
pub const PENDING: Map<u64, PendingForwarder> = Map::new("pending");
pub const NEXT_REPLY_ID: Item<u64> = Item::new("next_reply_id");
