//! XDG-style paths, used identically on macOS and Linux.

use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct Paths {
    #[allow(dead_code)] // read by config loading in milestone 2
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    /// Resolve paths, honouring `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME` and an
    /// explicit `--config` override.
    pub fn resolve(config_override: Option<PathBuf>) -> Result<Self> {
        let config_file = match config_override {
            Some(p) => p,
            None => xdg_dir("XDG_CONFIG_HOME", ".config")?
                .join("mattertui")
                .join("config.toml"),
        };
        let data_dir = xdg_dir("XDG_DATA_HOME", ".local/share")?.join("mattertui");
        Ok(Self {
            config_file,
            data_dir,
        })
    }

    /// The matter-controller snapshot (fabric keys + paired nodes).
    pub fn store(&self) -> PathBuf {
        self.data_dir.join("controller.bin")
    }

    pub fn roots_dir(&self) -> PathBuf {
        self.data_dir.join("roots")
    }

    pub fn paa_dir(&self) -> PathBuf {
        self.roots_dir().join("paa")
    }

    pub fn cd_dir(&self) -> PathBuf {
        self.roots_dir().join("cd")
    }

    /// Create the data dir (mode 0700: it holds private keys).
    pub fn ensure_data_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.data_dir)
            .with_context(|| format!("creating {}", self.data_dir.display()))?;
        fs::set_permissions(&self.data_dir, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("setting permissions on {}", self.data_dir.display()))?;
        Ok(())
    }

    /// Take the single-instance lock. Only one process may use the
    /// controller identity at a time; two would cancel each other's
    /// subscriptions. The lock is released when the returned guard drops.
    pub fn lock(&self) -> Result<InstanceLock> {
        self.ensure_data_dir()?;
        let path = self.data_dir.join("controller.lock");
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(InstanceLock { _file: file }),
            Err(fs::TryLockError::WouldBlock) => {
                bail!("mattertui is already running (lock held on {})", path.display())
            }
            Err(fs::TryLockError::Error(e)) => {
                Err(e).with_context(|| format!("locking {}", path.display()))
            }
        }
    }
}

/// Held for the process lifetime; dropping it releases the lock.
pub struct InstanceLock {
    _file: File,
}

fn xdg_dir(var: &str, home_relative: &str) -> Result<PathBuf> {
    if let Some(v) = std::env::var_os(var).filter(|v| !v.is_empty()) {
        let p = PathBuf::from(v);
        // The XDG spec says relative values must be ignored.
        if p.is_absolute() {
            return Ok(p);
        }
    }
    let home = std::env::home_dir().context("cannot determine home directory")?;
    Ok(home.join(home_relative))
}

/// Expand a leading `~/` in user-supplied paths.
#[allow(dead_code)] // used by config in milestone 2
pub fn expand_tilde(p: &Path) -> PathBuf {
    match (p.strip_prefix("~"), std::env::home_dir()) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => p.to_path_buf(),
    }
}
