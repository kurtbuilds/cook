use std::fs;

use serde::{Deserialize, Serialize};

use crate::service::unit::{RequiredWorkingDirectory, ServiceOwner};
use crate::{Error, FromKdl, Modification, ModificationOverSsh, Rule, RuleOverSsh};

#[cfg(feature = "ssh")]
use crate::sh_single_quote;

#[cfg(feature = "ssh")]
use crate::service::manager::{Platform, UnitKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    pub service_file_content: String,
    pub start: bool,
    pub owner: Option<String>,
    /// Content of the `.timer` unit to install alongside the service, if any.
    ///
    /// Populated either by copying a timer file (`timer=path`) or by generating
    /// one from an `on_calendar=...` schedule.
    pub timer_file_content: Option<String>,
    /// `WorkingDirectory=` read out of the unit file, when it names a path cook
    /// can create. systemd fails the unit at start time if this directory does
    /// not exist, so installing the service also means ensuring the directory
    /// and the ownership the unit's `User=`/`Group=` need on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<RequiredWorkingDirectory>,
}

/// Build the content of a `.timer` unit that triggers `{name}.service` on the
/// given `OnCalendar=` schedule (e.g. `Mon..Fri 18:00 America/New_York`).
///
/// `persistent` maps to `Persistent=`: when true (the default), a run missed
/// while the host was down fires on the next boot.
fn generate_timer_file_content(name: &str, on_calendar: &str, persistent: bool) -> String {
    let persistent_line = if persistent { "Persistent=true\n" } else { "" };
    format!(
        "# Managed by ser\n\
         [Unit]\n\
         Description=Timer for {name}\n\
         \n\
         [Timer]\n\
         OnCalendar={on_calendar}\n\
         {persistent_line}\
         \n\
         [Install]\n\
         WantedBy=timers.target\n"
    )
}

impl FromKdl for ServiceSpec {
    fn kdl_keywords() -> &'static [&'static str] {
        &["service"]
    }
    fn add_rules_to_state(state: &mut crate::State, node: &kdl::KdlNode, context: &crate::Context) {
        let mut entries = node.entries().iter();
        let name = entries.next().unwrap().expect_str().to_string();
        let service_file_path = entries.next().unwrap().expect_str();
        let path = context.local_path(service_file_path);
        let service_file_content = fs::read_to_string(path).expect("Failed to read service file");
        let mut start = true;
        let mut owner = None;
        let mut timer_file: Option<String> = None;
        let mut on_calendar: Option<String> = None;
        let mut persistent = true;
        while let Some(e) = entries.next() {
            match e.name().expect("Failed to get node name").value() {
                "start" => start = e.value().as_bool().expect("Value for start is not a bool"),
                "owner" => owner = Some(e.expect_str().to_string()),
                "timer" => {
                    let timer_path = context.local_path(e.expect_str());
                    let content = fs::read_to_string(timer_path).expect("Failed to read timer file");
                    timer_file = Some(content);
                }
                "on_calendar" => on_calendar = Some(e.expect_str().to_string()),
                "persistent" => persistent = e.value().as_bool().expect("Value for persistent is not a bool"),
                z => panic!("Unexpected option for service: {}", z),
            }
        }

        let timer_file_content = match (timer_file, on_calendar) {
            (Some(_), Some(_)) => {
                panic!(
                    "service {}: specify only one of `timer` or `on_calendar`, not both",
                    name
                )
            }
            (Some(content), None) => Some(content),
            (None, Some(schedule)) => Some(generate_timer_file_content(&name, &schedule, persistent)),
            (None, None) => None,
        };

        let working_directory = crate::service::unit::required_working_directory(&service_file_content);

        state.add_rule(ServiceSpec {
            name,
            service_file_content,
            start,
            owner,
            timer_file_content,
            working_directory,
        });
    }
}

impl Rule for ServiceSpec {
    fn kind(&self) -> &'static str {
        "service"
    }

    fn identifier(&self) -> &str {
        &self.name
    }

    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn crate::RuleOverSsh> {
        Some(self)
    }

    fn check(&self) -> Result<Vec<Box<dyn crate::Modification>>, crate::Error> {
        todo!()
    }
}

#[derive(Debug, Serialize)]
pub enum ServiceChange {
    MissingWorkingDirectory(MissingWorkingDirectory),
    WrongWorkingDirectoryOwner(WrongWorkingDirectoryOwner),
    NewService(NewService),
}

/// The unit's `WorkingDirectory=` does not exist on the target. systemd refuses
/// to start a unit whose working directory is missing (unless it is prefixed
/// with `-`), so cook creates it as part of applying the service rule.
#[derive(Debug, Serialize)]
pub struct MissingWorkingDirectory {
    /// Name of the service the directory belongs to, for readable output.
    pub service: String,
    pub directory: RequiredWorkingDirectory,
}

/// The unit's `WorkingDirectory=` exists but is not owned by the user the unit
/// runs as, which leaves the service unable to write to it.
#[derive(Debug, Serialize)]
pub struct WrongWorkingDirectoryOwner {
    pub service: String,
    pub path: String,
    pub owner: ServiceOwner,
    /// Current `user:group` on the target, for readable output.
    pub current: String,
}

#[derive(Debug, Serialize)]
pub struct NewService {
    pub name: String,
    pub service_file_content: String,
    pub service_file_content_sha256: String,
    pub start: bool,
    pub timer_file_content: Option<String>,
    pub timer_file_content_sha256: Option<String>,
}

#[cfg(feature = "ssh")]
fn sha256_hex(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl RuleOverSsh for ServiceSpec {
    async fn check_ssh(&self, session: &openssh::Session) -> Result<Vec<Box<dyn Modification>>, Error> {
        let manager = Platform::detect(session).await?.service_manager();

        let service_file_path = manager.unit_path(&self.name, UnitKind::Service);
        let local_service_sha256 = sha256_hex(&self.service_file_content);
        let remote_service_sha256 = manager.remote_checksum(session, &service_file_path).await?;
        // The service needs (re)writing if it's missing or its content differs.
        let service_changed = remote_service_sha256.as_deref() != Some(local_service_sha256.as_str());

        // Mirror the same check for the optional timer unit.
        let local_timer_sha256 = self.timer_file_content.as_deref().map(sha256_hex);
        let timer_changed = if self.timer_file_content.is_some() {
            let timer_file_path = manager.unit_path(&self.name, UnitKind::Timer);
            let remote_timer_sha256 = manager.remote_checksum(session, &timer_file_path).await?;
            remote_timer_sha256.as_deref() != local_timer_sha256.as_deref()
        } else {
            false
        };

        let mut changes: Vec<Box<dyn Modification>> = Vec::new();

        // The working directory is checked independently of the unit files: it
        // can go missing (or be left root-owned by an earlier run) on a host
        // whose units are already up to date, and it has to be right *before*
        // the unit is enabled, so this change is ordered ahead of the install.
        if let Some(directory) = &self.working_directory {
            match remote_owner(session, &directory.path).await? {
                None => changes.push(Box::new(ServiceChange::MissingWorkingDirectory(
                    MissingWorkingDirectory {
                        service: self.name.clone(),
                        directory: directory.clone(),
                    },
                ))),
                Some(current) => {
                    if let Some(owner) = &directory.owner
                        && !current.satisfies(owner)
                    {
                        changes.push(Box::new(ServiceChange::WrongWorkingDirectoryOwner(
                            WrongWorkingDirectoryOwner {
                                service: self.name.clone(),
                                path: directory.path.clone(),
                                owner: owner.clone(),
                                current: format!("{}:{}", current.user_name, current.group_name),
                            },
                        )));
                    }
                }
            }
        }

        if service_changed || timer_changed {
            changes.push(Box::new(ServiceChange::NewService(NewService {
                name: self.name.clone(),
                service_file_content: self.service_file_content.clone(),
                service_file_content_sha256: local_service_sha256,
                start: self.start,
                timer_file_content: self.timer_file_content.clone(),
                timer_file_content_sha256: local_timer_sha256,
            })));
        }

        Ok(changes)
    }
}

/// Ownership of a remote path, in both the name and numeric forms — a unit may
/// write `User=app` or `User=1001`, and either has to compare equal.
#[cfg(feature = "ssh")]
struct RemoteOwner {
    user_name: String,
    group_name: String,
    uid: String,
    gid: String,
}

#[cfg(feature = "ssh")]
impl RemoteOwner {
    /// Whether the current ownership already satisfies what the unit needs.
    ///
    /// A unit that sets only `User=` leaves the group to that user's login
    /// group, which cook cannot resolve locally — the group is left alone in
    /// that case, and `chown user:` sets it correctly if the directory is ever
    /// created or re-owned.
    fn satisfies(&self, owner: &ServiceOwner) -> bool {
        let user_ok = self.user_name == owner.user || self.uid == owner.user;
        let group_ok = owner
            .group
            .as_ref()
            .is_none_or(|g| &self.group_name == g || &self.gid == g);
        user_ok && group_ok
    }
}

/// Ownership of `path` on the remote host, or `None` if it is not a directory
/// there (missing, or something else in its place — `mkdir -p` reports which).
#[cfg(feature = "ssh")]
async fn remote_owner(session: &openssh::Session, path: &str) -> Result<Option<RemoteOwner>, Error> {
    // One round-trip for existence and ownership. `test -d` first so a
    // non-directory is reported as absent rather than as a wrong owner.
    let script = format!("test -d {p} && stat -c '%U %G %u %g' {p}", p = sh_single_quote(path));
    let output = session.command("sh").arg("-c").arg(&script).output().await?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8(output.stdout)?;
    let mut fields = stdout.split_whitespace();
    let (Some(user_name), Some(group_name), Some(uid), Some(gid)) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err(anyhow::anyhow!("could not read ownership of {path}: unexpected `stat` output {stdout:?}").into());
    };
    Ok(Some(RemoteOwner {
        user_name: user_name.to_string(),
        group_name: group_name.to_string(),
        uid: uid.to_string(),
        gid: gid.to_string(),
    }))
}

/// Give `path` the ownership the unit's processes need.
///
/// A unit that sets only `User=` gets `chown user:`, whose trailing colon tells
/// `chown` to use that user's login group — the same group systemd would run
/// the process under.
#[cfg(feature = "ssh")]
async fn chown(session: &openssh::Session, path: &str, owner: &ServiceOwner) -> Result<(), Error> {
    let spec = format!("{}:{}", owner.user, owner.group.as_deref().unwrap_or_default());
    let status = session.command("chown").arg(&spec).arg(path).status().await?;
    if !status.success() {
        return Err(anyhow::anyhow!("failed to set ownership of {path} to {spec}").into());
    }
    Ok(())
}

impl Modification for ServiceChange {
    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn ModificationOverSsh> {
        Some(self)
    }

    fn apply(&self) -> Result<(), Error> {
        todo!()
    }

    fn fmt_human_readable(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceChange::MissingWorkingDirectory(missing) => write!(
                f,
                "create working directory {} for service {}",
                missing.directory.path, missing.service
            ),
            ServiceChange::WrongWorkingDirectoryOwner(wrong) => write!(
                f,
                "chown working directory {} of service {} from {} to {}:{}",
                wrong.path,
                wrong.service,
                wrong.current,
                wrong.owner.user,
                wrong.owner.group.as_deref().unwrap_or_default()
            ),
            ServiceChange::NewService(service) => write!(f, "new service {}", service.name),
        }
    }

    fn fmt_json(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fmt_human_readable(f)
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl ModificationOverSsh for ServiceChange {
    async fn apply_ssh(&self, session: std::sync::Arc<openssh::Session>) -> Result<(), Error> {
        use openssh_sftp_client::{Sftp, SftpOptions};
        match self {
            ServiceChange::MissingWorkingDirectory(missing) => {
                let path = &missing.directory.path;
                let status = session.command("mkdir").arg("-p").arg(path).status().await?;
                if !status.success() {
                    return Err(anyhow::anyhow!(
                        "failed to create working directory {path} for service {}",
                        missing.service
                    )
                    .into());
                }
                // The directory is created by whoever cook connects as (root,
                // typically); hand it to the user the unit runs as.
                if let Some(owner) = &missing.directory.owner {
                    chown(&session, path, owner).await?;
                }
                Ok(())
            }
            ServiceChange::WrongWorkingDirectoryOwner(wrong) => {
                chown(&session, &wrong.path, &wrong.owner).await?;
                Ok(())
            }
            ServiceChange::NewService(service) => {
                let manager = Platform::detect(&session).await?.service_manager();

                let sftp = Sftp::from_clonable_session(session.clone(), SftpOptions::new()).await?;
                let service_path = manager.unit_path(&service.name, UnitKind::Service);
                let mut f = sftp.create(service_path).await?;
                f.write_all(service.service_file_content.as_bytes()).await?;
                f.close().await?;

                if let Some(timer_file_content) = &service.timer_file_content {
                    let timer_path = manager.unit_path(&service.name, UnitKind::Timer);
                    let mut f = sftp.create(timer_path).await?;
                    f.write_all(timer_file_content.as_bytes()).await?;
                    f.close().await?;
                }

                // Pick up the freshly written unit files (the "reload-daemon
                // dance" that `ser` used to do for us).
                manager.reload(&session).await?;

                if service.start {
                    // For a timer-backed service it's the timer that gets
                    // enabled — it triggers the service on schedule. Otherwise
                    // enable the service itself.
                    let kind = if service.timer_file_content.is_some() {
                        UnitKind::Timer
                    } else {
                        UnitKind::Service
                    };
                    manager.enable_now(&session, &service.name, kind).await?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::generate_timer_file_content;

    #[test]
    fn on_calendar_generates_a_complete_timer_unit() {
        let content = generate_timer_file_content("flex-orders-sync", "Mon..Fri 18:00 America/New_York", true);
        assert_eq!(
            content,
            "# Managed by ser\n\
             [Unit]\n\
             Description=Timer for flex-orders-sync\n\
             \n\
             [Timer]\n\
             OnCalendar=Mon..Fri 18:00 America/New_York\n\
             Persistent=true\n\
             \n\
             [Install]\n\
             WantedBy=timers.target\n"
        );
    }

    #[test]
    fn persistent_false_omits_the_persistent_line() {
        let content = generate_timer_file_content("flex-orders-sync", "Mon..Fri 18:00 America/New_York", false);
        assert!(!content.contains("Persistent"), "got: {content}");
        assert!(content.contains("OnCalendar=Mon..Fri 18:00 America/New_York\n"));
    }

    /// Comparing the unit's `User=`/`Group=` against what `stat` reports for the
    /// working directory. A mismatch is what triggers the chown, so a false
    /// match leaves the service unable to write and a false mismatch chowns on
    /// every run.
    #[cfg(feature = "ssh")]
    mod working_directory_owner {
        use super::super::RemoteOwner;
        use crate::service::unit::ServiceOwner;

        /// `/srv/app` owned by `app:app-grp`, uid/gid 1001/2002.
        fn current() -> RemoteOwner {
            RemoteOwner {
                user_name: "app".to_string(),
                group_name: "app-grp".to_string(),
                uid: "1001".to_string(),
                gid: "2002".to_string(),
            }
        }

        fn owner(user: &str, group: Option<&str>) -> ServiceOwner {
            ServiceOwner {
                user: user.to_string(),
                group: group.map(str::to_string),
            }
        }

        #[test]
        fn matching_names_satisfy() {
            assert!(current().satisfies(&owner("app", Some("app-grp"))));
        }

        #[test]
        fn a_unit_naming_ids_instead_of_names_satisfies() {
            assert!(current().satisfies(&owner("1001", Some("2002"))));
            // Mixing the two forms is still the same owner.
            assert!(current().satisfies(&owner("app", Some("2002"))));
        }

        #[test]
        fn a_different_user_or_group_does_not_satisfy() {
            assert!(!current().satisfies(&owner("root", Some("app-grp"))));
            assert!(!current().satisfies(&owner("app", Some("root"))));
            // A uid that is not this owner's must not match by coincidence.
            assert!(!current().satisfies(&owner("1002", None)));
        }

        #[test]
        fn an_unset_group_leaves_the_group_alone() {
            // Without `Group=` systemd uses the user's login group, which cook
            // cannot resolve locally — so any group satisfies.
            assert!(current().satisfies(&owner("app", None)));
        }
    }
}
