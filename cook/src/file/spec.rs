use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

use kdl::KdlNode;
use serde::{Deserialize, Serialize};

use crate::{Context, Error, FromKdl, Modification, ModificationOverSsh, Rule, RuleOverSsh, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileContent {
    Content(Vec<u8>, String),
    Url(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSpec {
    pub path: PathBuf,
    pub mode: u32,
    pub content: FileContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<u32>, // UID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u32>, // GID
}

impl FileSpec {
    pub fn new(path: PathBuf, content: Vec<u8>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let sha256 = format!("{:x}", hasher.finalize());
        FileSpec {
            path,
            mode: 0o644,
            content: FileContent::Content(content, sha256),
            owner: None,
            group: None,
        }
    }

    pub fn new_copy(src: PathBuf, dst: PathBuf) -> Self {
        let content = fs::read(src).expect("Failed to read path");
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let sha256 = format!("{:x}", hasher.finalize());
        FileSpec {
            path: dst,
            mode: 0o644,
            content: FileContent::Content(content, sha256),
            owner: None,
            group: None,
        }
    }
}

impl FromKdl for FileSpec {
    fn kdl_keywords() -> &'static [&'static str] {
        &["file", "cp"]
    }

    fn add_rules_to_state(state: &mut State, node: &KdlNode, context: &Context) {
        let mut args = node.entries().iter();
        match node.name().value() {
            "file" => {
                let dst = PathBuf::from(args.next().unwrap().expect_str());
                let file = FileSpec::new(dst, Vec::new());
                state.add_rule(file);
            }
            "cp" => {
                let src = context.local_path(args.next().unwrap().expect_str());
                let dst_str = args.next().unwrap().expect_str();
                let mut dst = PathBuf::from(dst_str);
                if src.is_dir() {
                    for entry in walkdir::WalkDir::new(&src) {
                        let entry = entry.expect("Failed to read directory entry");
                        let entry_path = entry.path();
                        if entry_path.is_file() {
                            let relative_path = entry_path.strip_prefix(&src).expect("Path should be under src");
                            let target_path = dst.join(relative_path);
                            let file = FileSpec::new_copy(entry_path.to_path_buf(), target_path);
                            state.add_rule(file);
                        }
                    } // walk the dir recursively. for every file, create a new FileSpec
                } else {
                    if dst_str.ends_with('/') {
                        dst.push(src.file_name().expect("Must have a file name."));
                    }
                    let file = FileSpec::new_copy(src, dst);
                    state.add_rule(file);
                }
            }
            z => panic!("invalid node for File: {}", z),
        }
    }
}
impl Rule for FileSpec {
    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn crate::RuleOverSsh> {
        Some(self)
    }

    fn check(&self) -> Result<Vec<Box<dyn Modification>>, Error> {
        todo!()
    }

    fn identifier(&self) -> &str {
        self.path.to_str().unwrap()
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl RuleOverSsh for FileSpec {
    async fn check_ssh(&self, session: &openssh::Session) -> Result<Vec<Box<dyn Modification>>, Error> {
        let mut changes: Vec<Box<dyn Modification>> = Vec::new();
        match &self.content {
            FileContent::Content(_, sha256) => {
                let output = session
                    .command("sha256sum")
                    .arg(self.path.to_str().unwrap())
                    .output()
                    .await?;
                let output = String::from_utf8_lossy(&output.stdout);
                let remote_hash = output.split_whitespace().next().unwrap_or_default();
                if sha256 != remote_hash {
                    changes.push(Box::new(FileChange::MissingFile(MissingFile {
                        path: self.path.clone(),
                        content: self.content.clone(),
                        owner: self.owner.clone(),
                        group: self.group.clone(),
                        mode: self.mode.clone(),
                    })));
                }
            }
            FileContent::Url(_) => {
                let output = session
                    .command("test")
                    .arg("-f")
                    .arg(self.path.to_str().unwrap())
                    .output()
                    .await?;
                if !output.status.success() {
                    changes.push(Box::new(FileChange::MissingFile(MissingFile {
                        path: self.path.clone(),
                        content: self.content.clone(),
                        owner: self.owner.clone(),
                        group: self.group.clone(),
                        mode: self.mode.clone(),
                    })));
                }
            }
        }
        Ok(changes)
    }
}

#[derive(Debug, Serialize)]
pub struct MissingFile {
    path: PathBuf,
    #[serde(skip)]
    content: FileContent,
    owner: Option<u32>,
    group: Option<u32>,
    mode: u32,
}

// #[derive(Debug, Serialize)]
// pub struct MissingDirectory {
//     path: String,
//     owner: Option<u32>,
//     group: Option<u32>,
//     mode: u32,
// }

// #[derive(Debug, Serialize)]
// pub struct MissingSymlink {
//     path: String,
//     target: String,
//     owner: Option<u32>,
//     group: Option<u32>,
//     mode: u32,
// }

#[derive(Debug, Serialize)]
pub enum FileChange {
    MissingFile(MissingFile),
    // MissingDirectory(MissingDirectory),
    // MissingSymlink(MissingSymlink),
}

impl Modification for FileChange {
    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn ModificationOverSsh> {
        Some(self)
    }

    fn apply(&self) -> Result<(), Error> {
        // let path = Path::new(&rule.path);
        match self {
            FileChange::MissingFile { .. } => todo!(),
            // FileChange::MissingDirectory { .. } => todo!(),
            // FileChange::MissingSymlink { .. } => todo!(),
            // FileChange::WrongPermissions { path, mode } => todo!(),
            // FileChange::WrongOwner { path, mode } => todo!(),
            // FileChange::WrongGroup { path, mode } => todo!(),
        }
    }

    fn fmt_human_readable(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "file change")
    }

    fn fmt_json(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "file change")
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl ModificationOverSsh for FileChange {
    async fn apply_ssh(&self, session: std::sync::Arc<openssh::Session>) -> Result<(), Error> {
        use openssh_sftp_client::{Sftp, SftpOptions};
        match self {
            FileChange::MissingFile(file) => {
                session
                    .command("mkdir")
                    .arg("-p")
                    .arg(file.path.parent().unwrap().to_str().unwrap())
                    .status()
                    .await?;
                match &file.content {
                    FileContent::Content(content, _) => {
                        let sftp = Sftp::from_clonable_session(session.clone(), SftpOptions::new()).await?;
                        let mut f = sftp.create(file.path.to_str().unwrap()).await?;
                        f.write_all(&content).await?;
                        f.close().await?;
                    }
                    FileContent::Url(url) => {
                        // Download file using curl over SSH and save to the target path
                        session
                            .command("curl")
                            .arg("-L")
                            .arg("-o")
                            .arg(file.path.to_str().unwrap())
                            .arg(url)
                            .status()
                            .await?;
                    }
                }
            } // FileChange::MissingDirectory { .. } => todo!(),
              // FileChange::MissingSymlink { .. } => todo!(),
        }
        Ok(())
    }
}
