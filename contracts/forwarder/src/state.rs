use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::Item;

/// Per-forwarder config. Shared operational fields (skip relayer / entrypoint /
/// refund address / owner) are NOT stored here — they live on the parent factory
/// and are read at runtime via `query_wasm_smart`, so a single factory update
/// applies to every forwarder it created.
#[cw_serde]
pub struct Config {
    /// Parent factory that instantiated this forwarder. Source of shared config.
    pub factory: Addr,
    /// Route identity (immutable, baked into the instantiate2 salt by the factory).
    pub sender_addr: String,
    pub recipient_addr: String,
    pub dest_chain: String,
}

pub const CONFIG: Item<Config> = Item::new("config");
