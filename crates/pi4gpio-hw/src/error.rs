use thiserror::Error;

#[derive(Debug, Error)]
pub enum HwError {
    #[error("failed to open device: {0}")]
    OpenFailed(String),

    #[error("invalid pin or channel number: {0}")]
    InvalidChannel(u32),

    #[error("transfer failed: {0}")]
    TransferFailed(String),

    #[error("hardware operation not yet implemented")]
    NotImplemented,
}
