use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Unauthorized")]
    Unauthorized,

    /// A forwarder for this (sender, dest_chain, recipient) route already exists.
    #[error("ForwarderExists")]
    ForwarderExists,

    /// reply fired for a reply_id with no matching PENDING entry.
    #[error("PendingNotFound")]
    PendingNotFound,

    /// instantiate2 submsg returned no data to parse the contract address from.
    #[error("NoInstantiateData")]
    NoInstantiateData,

    /// The instantiated forwarder address differs from the deterministically
    /// predicted one — should be impossible; fail loudly.
    #[error("AddressMismatch: predicted {predicted}, actual {actual}")]
    AddressMismatch { predicted: String, actual: String },

    #[error("Instantiate2: {0}")]
    Instantiate2(String),

    #[error("ParseReply: {0}")]
    ParseReply(String),
}
