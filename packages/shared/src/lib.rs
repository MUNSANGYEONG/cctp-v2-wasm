//! Shared types between `forwarder-factory` and `forwarder`.
//!
//! The factory owns the canonical query interface (`QueryMsg`) and its response
//! shapes. The forwarder depends on this crate so it can query the factory's
//! shared config (`QueryMsg::Config {}` -> `FactoryConfigResponse`) without
//! re-declaring the JSON schema and risking drift between the two contracts.

use cosmwasm_schema::{cw_serde, QueryResponses};

/// Factory query interface. Re-exported by the factory crate as its `QueryMsg`,
/// and used by the forwarder to read shared config from its parent factory.
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    /// Shared configuration applied to every forwarder this factory created.
    #[returns(FactoryConfigResponse)]
    Config {},
    /// Deterministically predicted address for a (sender, dest_chain, recipient)
    /// route, regardless of whether the forwarder has been instantiated yet.
    #[returns(ForwarderAddressResponse)]
    ForwarderAddress {
        sender_addr: String,
        dest_chain: String,
        recipient_addr: String,
    },
    /// Paginated list of forwarders already created by this factory.
    #[returns(ForwardersResponse)]
    Forwarders {
        start_after: Option<String>,
        limit: Option<u32>,
    },
}

/// Init payload the factory sends when instantiating a forwarder via
/// `instantiate2`. Shared so the JSON shape can never drift between the
/// factory (producer) and the forwarder (consumer).
#[cw_serde]
pub struct ForwarderInstantiateMsg {
    /// The instantiating factory; the forwarder verifies this equals `info.sender`.
    pub factory: String,
    pub sender_addr: String,
    pub recipient_addr: String,
    pub dest_chain: String,
}

/// Shared config every forwarder reads from the factory at runtime.
#[cw_serde]
pub struct FactoryConfigResponse {
    pub owner: String,
    pub forwarder_code_id: u64,
    pub skip_relayer_addr: String,
    pub skip_entrypoint_addr: String,
}

#[cw_serde]
pub struct ForwarderAddressResponse {
    /// Bech32 address derived via instantiate2 (deterministic).
    pub address: String,
    /// Whether a forwarder has actually been instantiated at `address`.
    pub exists: bool,
}

#[cw_serde]
pub struct ForwarderInfo {
    pub address: String,
    pub sender_addr: String,
    pub recipient_addr: String,
    pub dest_chain: String,
    pub created_height: u64,
}

#[cw_serde]
pub struct ForwardersResponse {
    pub forwarders: Vec<ForwarderInfo>,
}
