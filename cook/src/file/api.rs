use std::{
    fs,
    path::{Path, PathBuf},
};

use sha2::Digest;

use crate::{
    add_to_state, drop_last_rule,
    file::spec::{FileContent, FileSpec, FileType},
};

pub fn cp(src: impl AsRef<str>, destination: impl AsRef<str>) -> File {
    let destination_str = destination.as_ref();
    let mut destination = Path::new(destination_str).to_owned();
    if destination_str.ends_with("/") {
        let path = Path::new(src.as_ref());
        destination.push(path.file_name().expect("no filename"));
    }
    File {
        destination: destination,
        src: Some(src.as_ref().to_string()),
        content: None,
        link: None,
    }
}

pub fn file(destination: impl AsRef<str>) -> File {
    File::new(destination)
}

#[derive(Debug)]
pub struct File {
    destination: PathBuf,
    pub content: Option<String>,
    pub src: Option<String>,
    pub link: Option<String>,
}

impl File {
    /// Can be a file or a directory
    pub fn new(destination: impl AsRef<str>) -> Self {
        let destination = Path::new(destination.as_ref()).to_owned();
        Self {
            destination,
            src: None,
            content: None,
            link: None,
        }
    }

    pub fn forget(self) {
        let dest = self.destination.clone();
        std::mem::drop(self);
        drop_last_rule(dest.to_str().unwrap());
    }

    /// build the spec
    pub fn build(&mut self) -> FileSpec {
        let file_type;
        if self.destination.to_str().unwrap().ends_with("/") {
            file_type = FileType::Directory;
        } else if let Some(src) = self.src.as_ref() {
            let Ok(mut file) = std::fs::File::open(src) else {
                panic!("File does not exist: {}", src);
            };
            let mut hasher = sha2::Sha256::new();
            std::io::copy(&mut file, &mut hasher).expect("failed to read source file");
            let hash = hasher.finalize();
            let sha256 = format!("{:x}", hash);
            let path = fs::canonicalize(src).expect("failed to canonicalize path");
            file_type = FileType::File {
                sha256: sha256,
                content: FileContent::Path(path),
            };
        } else if self.content.is_some() {
            let content = std::mem::take(&mut self.content).unwrap();
            let sha256 = format!("{:x}", sha2::Sha256::digest(content.as_bytes()));
            file_type = FileType::File {
                sha256,
                content: FileContent::Content(content.into_bytes()),
            };
        } else if let Some(link) = self.link.as_ref() {
            file_type = FileType::Symlink {
                target: link.to_string(),
            };
        } else {
            let sha256 = format!("{:x}", sha2::Sha256::digest(Vec::new()));
            file_type = FileType::File {
                sha256,
                content: FileContent::Content(Vec::new()),
            };
        };
        FileSpec {
            path: std::mem::take(&mut self.destination),
            mode: 0,
            file_type,
            owner: None,
            group: None,
        }
    }
}

impl Drop for File {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let spec = self.build();
            add_to_state(spec);
        }
    }
}
