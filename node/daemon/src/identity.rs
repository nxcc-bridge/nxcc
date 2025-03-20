use std::{fs, path::Path};

use libp2p::identity::{self, Keypair};
use tracing::{info, warn};

use crate::error::AppError;

/// Creates an ephemeral in-memory identity (not saved to disk).
pub fn create_ephemeral_identity() -> Keypair {
    info!("Creating an ephemeral in-memory identity");
    Keypair::generate_ed25519()
}

/// Get existing identity from disk, or create a new one if none exists.
/// If the file on disk is corrupted, it will be overwritten with a fresh identity.
pub fn get_or_create_identity(path: &Path) -> Result<Keypair, AppError> {
    if path.exists() {
        info!("Loading identity from {}", path.display());
        let key_data = fs::read(path)?;

        match identity::Keypair::from_protobuf_encoding(&key_data) {
            Ok(keypair) => Ok(keypair),
            Err(_) => {
                warn!("Failed to deserialize identity file; creating a new one");
                create_and_save_identity(path)
            }
        }
    } else {
        info!(
            "No existing identity file; creating a new one at {}",
            path.display()
        );
        create_and_save_identity(path)
    }
}

/// Create a brand-new identity keypair and save it to the given file path.
fn create_and_save_identity(path: &Path) -> Result<Keypair, AppError> {
    let keypair = identity::Keypair::generate_ed25519();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let key_data = keypair
        .to_protobuf_encoding()
        .map_err(|e| AppError::Identity(format!("Failed to serialize identity: {}", e)))?;

    fs::write(path, key_data)?;

    Ok(keypair)
}
