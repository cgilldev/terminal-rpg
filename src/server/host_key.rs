//! SSH host-key persistence with restrictive creation permissions.

use anyhow::{Context, Result};
use russh::keys::{PrivateKey, load_secret_key, ssh_key};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(super) fn load_or_create_host_key(path: &PathBuf) -> Result<PrivateKey> {
    if path.exists() {
        return load_secret_key(path, None).context("load SSH host key");
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).context("create SSH host-key directory")?;
    }
    let key = PrivateKey::random(&mut rand_10::rng(), ssh_key::Algorithm::Ed25519)
        .context("generate Ed25519 SSH host key")?;
    let encoded = key
        .to_openssh(ssh_key::LineEnding::LF)
        .context("encode SSH host key")?;
    write_private_key(path, encoded.as_bytes())?;
    Ok(key)
}

#[cfg(unix)]
fn write_private_key(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .context("create SSH host key")?;
    file.write_all(bytes).context("write SSH host key")
}

#[cfg(not(unix))]
fn write_private_key(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).context("write SSH host key")
}
