use std::{
    fs, io,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use sha2::Digest;

pub(crate) static FILE_SPEC: Mutex<Vec<FileSpec>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileType {
    File {
        sha256: Option<String>,
        content: FileContent,
    },
    Directory,
    Symlink {
        target: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileContent {
    Content(String),
    Path(PathBuf),
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

impl FileSpec {
    pub fn check(&self) -> io::Result<Vec<FileChange>> {
        let mut changes = vec![];
        if !self.path.exists() {
            changes.push(FileChange::Missing);
            return Ok(changes); // Skip other checks, we'll recreate from scratch
        }

        let metadata = fs::symlink_metadata(&self.path)?;

        // Type check
        let actual_type = if metadata.file_type().is_symlink() {
            let target = fs::read_link(&self.path)?.to_string_lossy().into_owned();
            FileType::Symlink { target }
        } else if metadata.is_file() {
            let mut file = std::fs::File::open(&self.path).expect("failed to open source file");
            let mut hasher = sha2::Sha256::new();
            std::io::copy(&mut file, &mut hasher).expect("failed to read source file");
            let hash = hasher.finalize();
            let sha256 = format!("{:x}", hash);
            let path = fs::canonicalize(&self.path).expect("failed to canonicalize path");
            FileType::File {
                sha256: Some(sha256),
                content: FileContent::Path(path.to_path_buf()),
            }
        } else if metadata.is_dir() {
            FileType::Directory
        } else {
            panic!("Unknown file");
        };

        match (&self.file_type, &actual_type) {
            (FileType::File { .. }, FileType::File { .. }) => {
                changes.push(FileChange::WrongContents)
            }
            (
                FileType::Symlink { target },
                FileType::Symlink {
                    target: actual_target,
                },
            ) => {
                if actual_target != target {
                    changes.push(FileChange::WrongSymlinkTarget {
                        actual: actual_target.clone(),
                        expected: target.clone(),
                    })
                }
            }
            (FileType::Directory, FileType::Directory) => {}
            _ => {
                changes.push(FileChange::WrongType {
                    actual: actual_type,
                });
                return Ok(changes); // Will need to delete and recreate
            }
        }
        // Ownership
        if let Some(expected_uid) = self.owner {
            if metadata.uid() != expected_uid {
                changes.push(FileChange::WrongOwner {
                    actual: metadata.uid(),
                    expected: expected_uid,
                });
            }
        }

        if let Some(expected_gid) = self.group {
            if metadata.gid() != expected_gid {
                changes.push(FileChange::WrongGroup {
                    actual: metadata.gid(),
                    expected: expected_gid,
                });
            }
        }
        Ok(changes)
    }

    pub fn apply_all(&self, changes: &[FileChange]) -> io::Result<()> {
        for change in changes {
            self.apply(change)?;
        }
        Ok(())
    }

    fn create_desired(&self, path: &Path) -> io::Result<()> {
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

        Ok(())
    }
    pub fn apply(&self, change: &FileChange) -> io::Result<()> {
        let path = Path::new(&self.path);

        match change {
            FileChange::Missing => {
                self.create_desired(path)?;
            }
            FileChange::WrongType { .. } => {
                fs::remove_file(path)
                    .or_else(|_| fs::remove_dir_all(path))
                    .ok(); // best effort
                self.create_desired(path)?;
            }
            FileChange::WrongPermissions { .. } => {
                fs::set_permissions(path, fs::Permissions::from_mode(self.mode))?;
            }
            FileChange::WrongOwner { expected, .. } | FileChange::WrongGroup { expected, .. } => {
                // let uid = self.owner.unwrap_or_else(|| fs::metadata(path)?.uid());
                // let gid = self.group.unwrap_or_else(|| fs::metadata(path)?.gid());
                // Requires root to succeed
                // nix::unistd::chown(
                //     path,
                //     Some(nix::unistd::Uid::from_raw(uid)),
                //     Some(nix::unistd::Gid::from_raw(gid)),
                // )
                // .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            }
            FileChange::WrongContents => {
                // let mut file = OpenOptions::new().write(true).truncate(true).open(path)?;
                // file.write_all(self.contents.as_ref().unwrap().as_bytes())?;
            }
            FileChange::WrongSymlinkTarget { .. } => {
                fs::remove_file(path)?;
                if let FileType::Symlink { target } = &self.file_type {
                    std::os::unix::fs::symlink(target, path)?;
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub enum FileChange {
    Missing,
    WrongType { actual: FileType },
    WrongPermissions { actual: u32, expected: u32 },
    WrongContents,
    WrongOwner { actual: u32, expected: u32 },
    WrongGroup { actual: u32, expected: u32 },
    WrongSymlinkTarget { actual: String, expected: String },
}
