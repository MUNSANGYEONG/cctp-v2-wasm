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

    #[error("IBCTransferFailed")]
    IBCTransferFailed,

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

}
