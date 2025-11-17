// use crate::{add_to_state, drop_last_rule};

pub fn cp(src: impl Into<String>, destination: impl Into<String>) -> File {
    File {
        destination: destination.into(),
        src: Some(src.into()),
        content: None,
        link: None,
    }
}

pub fn file(destination: impl Into<String>) -> File {
    File::new(destination.into())
}

#[derive(Debug)]
pub struct File {
    pub destination: String,
    pub content: Option<String>,
    pub src: Option<String>,
    pub link: Option<String>,
}

impl File {
    /// Can be a file or a directory
    pub fn new(destination: impl Into<String>) -> Self {
        Self {
            destination: destination.into(),
            src: None,
            content: None,
            link: None,
        }
    }

    // pub fn forget(self) {
    //     let dest = self.destination.clone();
    //     std::mem::drop(self);
    //     drop_last_rule(&dest);
    // }

    // /// build the spec
    // pub fn build(&mut self) -> FileSpec {
    //     let file_type;
    //     if self.destination.to_str().unwrap().ends_with("/") {
    //         file_type = FileType::Directory;
    //     } else if let Some(src) = self.src.as_ref() {
    //         let Ok(mut file) = std::fs::File::open(src) else {
    //             panic!("File does not exist: {}", src);
    //         };
    //         let mut hasher = sha2::Sha256::new();
    //         std::io::copy(&mut file, &mut hasher).expect("failed to read source file");
    //         let hash = hasher.finalize();
    //         let sha256 = format!("{:x}", hash);
    //         let path = fs::canonicalize(src).expect("failed to canonicalize path");
    //         file_type = FileType::File {
    //             sha256: sha256,
    //             content: FileContent::Path(path),
    //         };
    //     } else if self.content.is_some() {
    //         let content = std::mem::take(&mut self.content).unwrap();
    //         let sha256 = format!("{:x}", sha2::Sha256::digest(content.as_bytes()));
    //         file_type = FileType::File {
    //             sha256,
    //             content: FileContent::Content(content.into_bytes()),
    //         };
    //     } else if let Some(link) = self.link.as_ref() {
    //         file_type = FileType::Symlink {
    //             target: link.to_string(),
    //         };
    //     } else {
    //         let sha256 = format!("{:x}", sha2::Sha256::digest(Vec::new()));
    //         file_type = FileType::File {
    //             sha256,
    //             content: FileContent::Content(Vec::new()),
    //         };
    //     };
    //     FileSpec {
    //         path: std::mem::take(&mut self.destination),
    //         mode: 0,
    //         file_type,
    //         owner: None,
    //         group: None,
    //     }
    // }
}

// impl Drop for File {
//     fn drop(&mut self) {
//         if !std::thread::panicking() {
//             let spec = self.build();
//             add_to_state(spec);
//         }
//     }
// }
