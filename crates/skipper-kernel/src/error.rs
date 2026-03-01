//! Kernel-specific error types.

use skipper_types::error::SkipperError;
use thiserror::Error;

/// Kernel error type wrapping SkipperError with kernel-specific context.
#[derive(Error, Debug)]
pub enum KernelError {
    /// A wrapped SkipperError.
    #[error(transparent)]
    Skipper(#[from] SkipperError),

    /// The kernel failed to boot.
    #[error("Boot failed: {0}")]
    BootFailed(String),
}

/// Alias for kernel results.
pub type KernelResult<T> = Result<T, KernelError>;
