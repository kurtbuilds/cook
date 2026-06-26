use std::fs;

use serde::{Deserialize, Serialize};

use crate::{Error, FromKdl, Modification, ModificationOverSsh, Rule, RuleOverSsh};

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
                "persistent" => {
                    persistent = e.value().as_bool().expect("Value for persistent is not a bool")
                }
                z => panic!("Unexpected option for service: {}", z),
            }
        }

        let timer_file_content = match (timer_file, on_calendar) {
            (Some(_), Some(_)) => {
                panic!("service {}: specify only one of `timer` or `on_calendar`, not both", name)
            }
            (Some(content), None) => Some(content),
            (None, Some(schedule)) => {
                Some(generate_timer_file_content(&name, &schedule, persistent))
            }
            (None, None) => None,
        };

        state.add_rule(ServiceSpec {
            name,
            service_file_content,
            start,
            owner,
            timer_file_content,
        });
    }
}

impl Rule for ServiceSpec {
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
    NewService(NewService),
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

        if !service_changed && !timer_changed {
            return Ok(vec![]);
        }

        Ok(vec![Box::new(ServiceChange::NewService(NewService {
            name: self.name.clone(),
            service_file_content: self.service_file_content.clone(),
            service_file_content_sha256: local_service_sha256,
            start: self.start,
            timer_file_content: self.timer_file_content.clone(),
            timer_file_content_sha256: local_timer_sha256,
        }))])
    }
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
        write!(f, "new service")
    }

    fn fmt_json(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "new service")
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl ModificationOverSsh for ServiceChange {
    async fn apply_ssh(&self, session: std::sync::Arc<openssh::Session>) -> Result<(), Error> {
        use openssh_sftp_client::{Sftp, SftpOptions};
        match self {
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
        let content = generate_timer_file_content(
            "flex-orders-sync",
            "Mon..Fri 18:00 America/New_York",
            true,
        );
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
        let content =
            generate_timer_file_content("flex-orders-sync", "Mon..Fri 18:00 America/New_York", false);
        assert!(!content.contains("Persistent"), "got: {content}");
        assert!(content.contains("OnCalendar=Mon..Fri 18:00 America/New_York\n"));
    }
}
