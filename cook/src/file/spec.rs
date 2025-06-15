use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use kdl::KdlNode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Context, Error, File, Modification, ModificationOverSsh, Rule, RuleOverSsh, cp};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileType {
    File {
        sha256: String,
        content: FileContent,
    },
    Directory,
    Symlink {
        target: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileContent {
    Content(Vec<u8>),
    Path(PathBuf),
}

impl FileContent {
    pub fn content(&self) -> Vec<u8> {
        match self {
            FileContent::Content(content) => content.clone(),
            FileContent::Path(path) => fs::read(path).expect("failed to read file"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSpec {
    pub path: PathBuf,
    pub mode: u32,
    pub file_type: FileType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<u32>, // UID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u32>, // GID
}

impl Rule for FileSpec {
    fn downcast_ssh(&self) -> Option<&dyn crate::RuleOverSsh> {
        Some(self)
    }
    fn from_kdl(node: &KdlNode, context: &Context) -> Self {
        let root = &context.root;
        let mut args = node.entries().iter();
        match node.name().value() {
            "file" => {
                let name = args.next().unwrap();
                let name = name
                    .value()
                    .as_string()
                    .expect("name is not a string in file declaration");
                File::new(name).build()
            }
            "cp" => {
                let root = fs::canonicalize(root).unwrap();
                let src = args.next().unwrap();
                let src = root.join(
                    src.value()
                        .as_string()
                        .expect("source is not a string in cp declaration"),
                );
                let src = src.to_str().unwrap();
                let dst = args.next().unwrap();
                let dst = root.join(
                    dst.value()
                        .as_string()
                        .expect("destination is not a string in cp declaration"),
                );
                let dst = dst.to_str().unwrap();
                cp(src, dst).build()
            }
            z => panic!("invalid node for File: {}", z),
        }
    }

    fn check(&self) -> Result<Vec<Box<dyn Modification>>, Error> {
        todo!()
        // let mut changes: Vec<Box<dyn Modification>> = vec![];
        // if !self.path.exists() {
        //     let m = FileChange::Missing;
        //     changes.push(Box::new(m));
        //     return Ok(changes); // Skip other checks, we'll recreate from scratch
        // }

        // let metadata = fs::symlink_metadata(&self.path)?;

        // // Type check
        // let actual_type = if metadata.file_type().is_symlink() {
        //     let target = fs::read_link(&self.path)?.to_string_lossy().into_owned();
        //     FileType::Symlink { target }
        // } else if metadata.is_file() {
        //     let mut file = std::fs::File::open(&self.path).expect("failed to open source file");
        //     let mut hasher = sha2::Sha256::new();
        //     std::io::copy(&mut file, &mut hasher).expect("failed to read source file");
        //     let hash = hasher.finalize();
        //     let sha256 = format!("{:x}", hash);
        //     let path = fs::canonicalize(&self.path).expect("failed to canonicalize path");
        //     FileType::File {
        //         sha256,
        //         content: FileContent::Path(path.to_path_buf()),
        //     }
        // } else if metadata.is_dir() {
        //     FileType::Directory
        // } else {
        //     panic!("Unknown file");
        // };

        // match (&self.file_type, &actual_type) {
        //     (FileType::File { .. }, FileType::File { .. }) => {
        //         let m = FileChange::WrongContents;
        //         changes.push(Box::new(m))
        //     }
        //     (
        //         FileType::Symlink { target },
        //         FileType::Symlink {
        //             target: actual_target,
        //         },
        //     ) => {
        //         if actual_target != target {
        //             changes.push_boxed(FileChange::WrongSymlinkTarget {
        //                 actual: actual_target.clone(),
        //                 expected: target.clone(),
        //             })
        //         }
        //     }
        //     (FileType::Directory, FileType::Directory) => {}
        //     _ => {
        //         changes.push_boxed(FileChange::WrongType {
        //             actual: actual_type,
        //         });
        //         return Ok(changes); // Will need to delete and recreate
        //     }
        // }
        // // Ownership
        // if let Some(expected_uid) = self.owner {
        //     if metadata.uid() != expected_uid {
        //         changes.push_boxed(FileChange::WrongOwner {
        //             actual: metadata.uid(),
        //             expected: expected_uid,
        //         });
        //     }
        // }

        // if let Some(expected_gid) = self.group {
        //     if metadata.gid() != expected_gid {
        //         changes.push_boxed(FileChange::WrongGroup {
        //             actual: metadata.gid(),
        //             expected: expected_gid,
        //         });
        //     }
        // }
        // Ok(changes)
    }

    fn identifier(&self) -> &str {
        todo!()
    }
}

#[async_trait]
impl RuleOverSsh for FileSpec {
    async fn check_ssh(
        &self,
        session: &openssh::Session,
    ) -> Result<Vec<Box<dyn Modification>>, Error> {
        match &self.file_type {
            FileType::File { sha256, content } => {
                let output = session
                    .command("cat")
                    .arg(self.path.to_str().unwrap())
                    .output()
                    .await;
                if let Ok(output) = output {
                    let stdout = output.stdout;
                    let mut hasher = Sha256::new();
                    hasher.update(stdout);
                    let hash = hasher.finalize();
                    let hash = format!("{:x}", hash);
                    if sha256 == &hash {
                        return Ok(Vec::new());
                    }
                }
                return Ok(vec![Box::new(FileChange::MissingFile(MissingFile {
                    path: self.path.clone(),
                    content: content.content(),
                    sha256: sha256.clone(),
                    owner: self.owner.clone(),
                    group: self.group.clone(),
                    mode: self.mode.clone(),
                }))]);
            }
            FileType::Directory => {
                unimplemented!()
            }
            FileType::Symlink { .. } => {
                unimplemented!()
            }
        }
    }
}

impl FileSpec {
    // fn create_desired(&self, _path: &Path) -> io::Result<()> {
    // match &self.file_type {
    //     FileType::File => {
    //         let mut file = File::create(path)?;
    //         if let Some(contents) = &self.contents {
    //             file.write_all(contents.as_bytes())?;
    //         }
    //     }
    //     FileType::Directory => {
    //         fs::create_dir_all(path)?;
    //     }
    //     FileType::Symlink { target } => {
    //         std::os::unix::fs::symlink(target, path)?;
    //     }
    // }

    // fs::set_permissions(path, fs::Permissions::from_mode(self.mode))?;

    // if let (Some(uid), Some(gid)) = (self.owner, self.group) {
    //     nix::unistd::chown(
    //         path,
    //         Some(nix::unistd::Uid::from_raw(uid)),
    //         Some(nix::unistd::Gid::from_raw(gid)),
    //     )
    //     .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    // }

    // Ok(())
    // }
}

#[derive(Debug, Serialize)]
pub struct MissingFile {
    path: PathBuf,
    #[serde(skip)]
    content: Vec<u8>,
    sha256: String,
    owner: Option<u32>,
    group: Option<u32>,
    mode: u32,
}

#[derive(Debug, Serialize)]
pub enum FileChange {
    MissingFile(MissingFile),
    MissingDirectory {
        path: String,
        owner: Option<u32>,
        group: Option<u32>,
        mode: u32,
    },
    MissingSymlink {
        path: String,
        target: String,
        owner: Option<u32>,
        group: Option<u32>,
        mode: u32,
    },
    // WrongPermissions {
    //     path: String,
    //     mode: u32,
    // },
    // WrongOwner {
    //     path: String,
    //     mode: u32,
    // },
    // WrongGroup {
    //     path: String,
    //     mode: u32,
    // },
}

impl std::fmt::Display for FileChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "file change")
    }
}

impl Modification for FileChange {
    fn downcast_ssh(&self) -> Option<&dyn ModificationOverSsh> {
        Some(self)
    }

    fn apply(&self) -> Result<(), Error> {
        // let path = Path::new(&rule.path);
        match self {
            FileChange::MissingFile { .. } => todo!(),
            FileChange::MissingDirectory { .. } => todo!(),
            FileChange::MissingSymlink { .. } => todo!(),
            // FileChange::WrongPermissions { path, mode } => todo!(),
            // FileChange::WrongOwner { path, mode } => todo!(),
            // FileChange::WrongGroup { path, mode } => todo!(),
        }
    }
}

#[async_trait]
impl ModificationOverSsh for FileChange {
    async fn apply_ssh(&self, session: Arc<openssh::Session>) -> Result<(), Error> {
        let sftp = openssh_sftp_client::Sftp::from_clonable_session(
            session.clone(),
            openssh_sftp_client::SftpOptions::new(),
        )
        .await?;
        match self {
            FileChange::MissingFile(file) => {
                _ = session
                    .command("rm")
                    .arg(file.path.to_str().unwrap())
                    .stdout(openssh::Stdio::null())
                    .stderr(openssh::Stdio::null())
                    .status()
                    .await;
                session
                    .command("mkdir")
                    .arg("-p")
                    .arg(file.path.parent().unwrap().to_str().unwrap())
                    .status()
                    .await?;
                let mut f = sftp.create(file.path.to_str().unwrap()).await?;
                f.write_all(&file.content).await?;
                f.close().await?;
            }
            FileChange::MissingDirectory { .. } => todo!(),
            FileChange::MissingSymlink { .. } => todo!(),
            // FileChange::WrongPermissions { path, mode } => todo!(),
            // FileChange::WrongOwner { path, mode } => todo!(),
            // FileChange::WrongGroup { path, mode } => todo!(),
        }
        Ok(())
    }
}
