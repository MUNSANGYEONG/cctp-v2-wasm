use cosmwasm_std::StdError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("MemoEncodeError")]
    MemoEncodeError,

    #[error("NoFundsToForward")]
    NoFundsToForward,

    #[error("InvalidStatusTransition")]
    InvalidStatusTransition,

    #[error("DuplicateMintTxHash")]
    DuplicateMintTxHash,

    #[error("RequestNotFound")]
    RequestNotFound,

    #[error("InsufficientRequestFunds")]
    InsufficientRequestFunds,

    /// The forwarder could not read shared config from its parent factory.
    /// No fallback — the transaction reverts so funds are never forwarded with
    /// stale or missing routing parameters.
    #[error("FactoryQueryFailed: {0}")]
    FactoryQueryFailed(String),

    /// `instantiate` was not called by the declared factory address.
    #[error("FactoryMismatch")]
    FactoryMismatch,

    /// hook_data top-level key나 sent_asset 구조가 올바르지 않음.
    #[error("InvalidHookData")]
    InvalidHookData,

    /// sender_addr cannot be converted to a valid inj1 bech32 address.
    #[error("InvalidRefundAddress")]
    InvalidRefundAddress,
}
