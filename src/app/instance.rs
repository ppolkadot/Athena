//! Single-instance guard: an exclusive OS file lock held for the process lifetime.

use std::fs::{File, OpenOptions};
use std::path::Path;

use fs4::{FileExt, TryLockError};

use crate::shared::error::{AppError, Result};

/// Holds the `athena.lock` handle. The OS releases the lock when this drops or the process exits.
#[derive(Debug)]
pub struct InstanceGuard {
    #[allow(dead_code)] // kept open on purpose
    file: File,
}

/// # Errors
/// [`AppError::AlreadyRunning`] when another process holds the lock; [`AppError::Io`] otherwise.
pub fn acquire(dir: &Path) -> Result<InstanceGuard> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("athena.lock"))?;
    match FileExt::try_lock(&file) {
        Ok(()) => Ok(InstanceGuard { file }),
        Err(TryLockError::WouldBlock) => Err(AppError::AlreadyRunning),
        Err(TryLockError::Error(e)) => Err(AppError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_instance_is_rejected_while_first_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire(dir.path()).unwrap();
        assert!(matches!(acquire(dir.path()), Err(AppError::AlreadyRunning)));
        drop(first);
        assert!(acquire(dir.path()).is_ok());
    }
}
