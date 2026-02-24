use anyhow::{bail, Context, Result};
use iroh::SecretKey;
use std::fs;
use std::path::PathBuf;

const KEY_PATH: &str = ".local/share/punch/secret.key";
const KEY_LEN: usize = 32;

fn key_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("cannot determine home directory")?;
    Ok(home.join(KEY_PATH))
}

pub fn load_or_generate() -> Result<SecretKey> {
    let path = key_path()?;

    if path.exists() {
        let bytes = fs::read(&path).context("failed to read secret key")?;
        if bytes.len() != KEY_LEN {
            bail!(
                "secret key file is {} bytes, expected {}",
                bytes.len(),
                KEY_LEN
            );
        }
        let bytes: [u8; KEY_LEN] = bytes.try_into().unwrap();
        Ok(SecretKey::from_bytes(&bytes))
    } else {
        let key = SecretKey::generate(&mut rand::rng());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("failed to create key directory")?;
        }
        #[cfg(unix)]
        {
            use std::fs::OpenOptions;
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .context("failed to create secret key file")?;
            f.write_all(&key.to_bytes())
                .context("failed to write secret key")?;
        }
        #[cfg(not(unix))]
        {
            fs::write(&path, key.to_bytes()).context("failed to write secret key")?;
        }
        eprintln!("secret key created at {}", path.display());
        Ok(key)
    }
}
