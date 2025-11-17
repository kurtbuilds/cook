use std::fs;

use serde::{Deserialize, Serialize};

use crate::{Error, FromKdl, Modification, ModificationOverSsh, Rule, RuleOverSsh};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    pub service_file_content: String,
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

        state.add_rule(ServiceSpec {
            name,
            service_file_content,
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
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl RuleOverSsh for ServiceSpec {
    async fn check_ssh(&self, session: &openssh::Session) -> Result<Vec<Box<dyn Modification>>, Error> {
        // Check if systemd service exists
        let service_exists = session
            .command("systemctl")
            .arg("list-unit-files")
            .arg(&format!("{}.service", self.name))
            .output()
            .await?
            .status
            .success();

        if !service_exists {
            // Service doesn't exist, need to create it
            let service_file_content_sha256 = {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(self.service_file_content.as_bytes());
                format!("{:x}", hasher.finalize())
            };

            return Ok(vec![Box::new(ServiceChange::NewService(NewService {
                name: self.name.clone(),
                service_file_content: self.service_file_content.clone(),
                service_file_content_sha256,
            }))]);
        }

        // Service exists, check if content matches
        let service_file_path = format!("/etc/systemd/system/{}.service", self.name);
        let sha256_output = session.command("sha256sum").arg(&service_file_path).output().await?;

        if !sha256_output.status.success() {
            return Err(anyhow::anyhow!("Failed to get sha256sum of {}", service_file_path).into());
        }

        let remote_sha256 = String::from_utf8(sha256_output.stdout)?
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();

        let local_sha256 = {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(self.service_file_content.as_bytes());
            format!("{:x}", hasher.finalize())
        };

        if remote_sha256 != local_sha256 {
            // Content differs, need to update
            return Ok(vec![Box::new(ServiceChange::NewService(NewService {
                name: self.name.clone(),
                service_file_content: self.service_file_content.clone(),
                service_file_content_sha256: local_sha256,
            }))]);
        } else {
            Ok(vec![])
        }
    }
}

impl Modification for ServiceChange {
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
                let sftp = Sftp::from_clonable_session(session.clone(), SftpOptions::new()).await?;
                let file_path = format!("/etc/systemd/system/{}.service", service.name);
                let mut f = sftp.create(file_path).await?;
                f.write_all(service.service_file_content.as_bytes()).await?;
                f.close().await?;
                let success = session
                    .command("ser")
                    .arg("restart")
                    .arg(&service.name)
                    .output()
                    .await?
                    .status
                    .success();
                if !success {
                    Err(anyhow::anyhow!("Failed to restart service").into_boxed_dyn_error())
                } else {
                    Ok(())
                }
            }
        }
    }
}
