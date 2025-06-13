use std::{
    fs,
    path::{Path, PathBuf},
};

use sha2::Digest;

use crate::file::spec::{FILE_SPEC, FileContent, FileSpec, FileType};

pub fn cp(src: impl AsRef<str>, destination: impl AsRef<str>) -> File {
    // let path = Path::new(src.as_ref());
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
        std::mem::drop(self);
        FILE_SPEC.lock().unwrap().pop();
    }

    pub(crate) fn create_spec(&mut self) -> FileSpec {
        let file_type;
        if self.destination.to_str().unwrap().ends_with("/") {
            file_type = FileType::Directory;
        } else if let Some(src) = self.src.as_ref() {
            let mut file = std::fs::File::open(src).expect("failed to open source file");
            let mut hasher = sha2::Sha256::new();
            std::io::copy(&mut file, &mut hasher).expect("failed to read source file");
            let hash = hasher.finalize();
            let sha256 = format!("{:x}", hash);
            let path = fs::canonicalize(src).expect("failed to canonicalize path");
            file_type = FileType::File {
                sha256: Some(sha256),
                content: FileContent::Path(path),
            };
        } else if self.content.is_some() {
            let content = std::mem::take(&mut self.content).unwrap();
            file_type = FileType::File {
                sha256: None,
                content: FileContent::Content(content),
            };
        } else if let Some(link) = self.link.as_ref() {
            file_type = FileType::File {
                sha256: None,
                content: FileContent::Content(link.to_string()),
            };
        } else {
            file_type = FileType::File {
                sha256: None,
                content: FileContent::Content("".to_string()),
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
        let spec = self.create_spec();
        FILE_SPEC.lock().unwrap().push(spec);
    }
}
