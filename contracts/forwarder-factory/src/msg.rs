use cosmwasm_schema::cw_serde;

/// The factory's query interface is the shared `QueryMsg` (so the forwarder can
/// depend on it without a circular crate dependency).
pub use forwarder_factory_shared::QueryMsg;

#[cw_serde]
pub struct InstantiateMsg {
    /// Defaults to the instantiating sender when omitted.
    pub owner: Option<String>,
    /// code_id of the uploaded forwarder contract used for instantiate2.
    pub forwarder_code_id: u64,
    pub skip_relayer_addr: String,
    pub skip_entrypoint_addr: String,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Deterministically create a forwarder for a (sender, dest_chain, recipient)
    /// route. Owner-only. Idempotent-guarded by the route salt.
    CreateForwarder {
        sender_addr: String,
        dest_chain: String,
        recipient_addr: String,
    },
    /// Update shared config. Owner-only. Changing `forwarder_code_id` changes the
    /// predicted address of *future* forwarders (existing ones are unaffected).
    UpdateConfig {
        owner: Option<String>,
        forwarder_code_id: Option<u64>,
        skip_relayer_addr: Option<String>,
        skip_entrypoint_addr: Option<String>,
    },
}

/// No-op migrate payload (cw2 versioning hook).
#[cw_serde]
pub struct MigrateMsg {}
