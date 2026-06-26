//! Platform-aware service management.
//!
//! `ser` (the homegrown CLI) abstracts over `systemctl` (Linux/systemd) and
//! `launchctl` (macOS/launchd). This module reimplements that abstraction for
//! cook's use over SSH, with one important subtlety: the platform we care about
//! is the OS of the host *executing* the commands (the remote end of the SSH
//! session), not the OS running cook. So platform detection happens over the
//! wire via `uname`, never via `cfg!(target_os = ...)`.
//!
//! The [`ServiceManager`] trait is the composable seam: each platform owns its
//! own unit-file locations and control commands, so adding/altering a platform
//! is a single self-contained `impl`.

#[cfg(feature = "ssh")]
use crate::Error;

/// The kind of unit file being managed. Names map to the platform's conventions
/// in [`ServiceManager::unit_path`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitKind {
    Service,
    Timer,
}

/// Operating system of a (possibly remote) host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Macos,
}

#[cfg(feature = "ssh")]
impl Platform {
    /// Detect the platform of the host on the far end of an SSH session.
    ///
    /// This is the OS that will *execute* the service commands, which is
    /// distinct from the OS of the machine running cook — hence we ask the
    /// remote via `uname -s` rather than consulting a compile-time `cfg`.
    pub async fn detect(session: &openssh::Session) -> Result<Platform, Error> {
        let output = session.command("uname").arg("-s").output().await?;
        if !output.status.success() {
            return Err(anyhow::anyhow!("failed to detect remote platform via `uname -s`").into());
        }
        let os = String::from_utf8(output.stdout)?.trim().to_string();
        match os.as_str() {
            "Linux" => Ok(Platform::Linux),
            "Darwin" => Ok(Platform::Macos),
            other => Err(anyhow::anyhow!("unsupported remote platform: {other}").into()),
        }
    }

    /// The [`ServiceManager`] appropriate for this platform.
    pub fn service_manager(self) -> Box<dyn ServiceManager> {
        match self {
            Platform::Linux => Box::new(Systemd),
            Platform::Macos => Box::new(Launchd),
        }
    }
}

/// Platform-specific service management over an SSH session.
///
/// Every operation that differs between systemd and launchd lives behind this
/// trait, so [`crate::service::spec`] can orchestrate installs without knowing
/// which platform it is talking to.
#[cfg(feature = "ssh")]
#[async_trait::async_trait]
pub trait ServiceManager: Send + Sync {
    /// Absolute path on the remote where a unit file of the given kind/name
    /// should be written.
    fn unit_path(&self, name: &str, kind: UnitKind) -> String;

    /// The sha256 of a remote file, or `None` if it does not exist. Used to
    /// decide whether a unit file needs (re)writing. The hashing command itself
    /// differs by platform (`sha256sum` vs `shasum`).
    async fn remote_checksum(&self, session: &openssh::Session, path: &str) -> Result<Option<String>, Error>;

    /// Re-read unit definitions after unit files change. This is the
    /// "reload-daemon dance" that `ser` performs automatically (systemd
    /// `daemon-reload`); on platforms that don't need it this is a no-op.
    async fn reload(&self, session: &openssh::Session) -> Result<(), Error>;

    /// Enable a unit so that it runs now and on boot.
    async fn enable_now(&self, session: &openssh::Session, name: &str, kind: UnitKind) -> Result<(), Error>;
}

/// systemd-backed service management (Linux).
#[cfg(feature = "ssh")]
pub struct Systemd;

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl ServiceManager for Systemd {
    fn unit_path(&self, name: &str, kind: UnitKind) -> String {
        let ext = match kind {
            UnitKind::Service => "service",
            UnitKind::Timer => "timer",
        };
        format!("/etc/systemd/system/{name}.{ext}")
    }

    async fn remote_checksum(&self, session: &openssh::Session, path: &str) -> Result<Option<String>, Error> {
        let output = session.command("sha256sum").arg(path).output().await?;
        if !output.status.success() {
            // A failed `sha256sum` (e.g. missing file) means "not present".
            return Ok(None);
        }
        let sha = String::from_utf8(output.stdout)?
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        Ok(Some(sha))
    }

    async fn reload(&self, session: &openssh::Session) -> Result<(), Error> {
        let success = session
            .command("systemctl")
            .arg("daemon-reload")
            .output()
            .await?
            .status
            .success();
        if !success {
            return Err(anyhow::anyhow!("`systemctl daemon-reload` failed").into());
        }
        Ok(())
    }

    async fn enable_now(&self, session: &openssh::Session, name: &str, kind: UnitKind) -> Result<(), Error> {
        let unit = self.unit_name(name, kind);
        let success = session
            .command("systemctl")
            .arg("enable")
            .arg("--now")
            .arg(&unit)
            .output()
            .await?
            .status
            .success();
        if !success {
            return Err(anyhow::anyhow!("`systemctl enable --now {unit}` failed").into());
        }
        Ok(())
    }
}

#[cfg(feature = "ssh")]
impl Systemd {
    /// The unit name as systemctl refers to it, e.g. `caddy.timer`.
    fn unit_name(&self, name: &str, kind: UnitKind) -> String {
        match kind {
            UnitKind::Service => format!("{name}.service"),
            UnitKind::Timer => format!("{name}.timer"),
        }
    }
}

/// launchd-backed service management (macOS).
///
/// cook currently models services as systemd unit files (`.service`/`.timer`),
/// which launchd cannot consume — it expects `.plist` files with a different
/// schema. Until that translation exists, the launchd manager refuses installs
/// with a clear error rather than writing systemd files into `LaunchDaemons`
/// where they would silently never run. The trait seam means wiring up real
/// launchd support later is confined to this `impl`.
#[cfg(feature = "ssh")]
pub struct Launchd;

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl ServiceManager for Launchd {
    fn unit_path(&self, name: &str, _kind: UnitKind) -> String {
        format!("/Library/LaunchDaemons/{name}.plist")
    }

    async fn remote_checksum(&self, _session: &openssh::Session, _path: &str) -> Result<Option<String>, Error> {
        Err(launchd_unsupported())
    }

    async fn reload(&self, _session: &openssh::Session) -> Result<(), Error> {
        Err(launchd_unsupported())
    }

    async fn enable_now(&self, _session: &openssh::Session, _name: &str, _kind: UnitKind) -> Result<(), Error> {
        Err(launchd_unsupported())
    }
}

#[cfg(feature = "ssh")]
fn launchd_unsupported() -> Error {
    anyhow::anyhow!(
        "managing services on macOS (launchd) is not yet supported: cook generates systemd \
         unit files, which launchd cannot run. Target a Linux host, or add launchd/plist \
         support to the Launchd service manager."
    )
    .into()
}
