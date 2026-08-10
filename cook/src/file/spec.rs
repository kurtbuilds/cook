use globset::{Glob, GlobSet, GlobSetBuilder};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use kdl::{KdlEntry, KdlNode, KdlValue};
use serde::{Deserialize, Serialize};

use crate::{Context, Error, FromKdl, Modification, ModificationOverSsh, Rule, RuleOverSsh, State};

#[cfg(feature = "ssh")]
use crate::sh_single_quote;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileContent {
    Content(Vec<u8>, String),
    Url(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSpec {
    pub path: PathBuf,
    /// Permission bits to enforce, e.g. `Some(0o755)` for an executable.
    ///
    /// `None` means the mode is not cook's to manage: the file is uploaded and
    /// left at whatever mode it lands at (sftp's default for a new file, the
    /// existing mode for one being replaced). Most files don't care, and a
    /// default of 0644 here would silently strip the exec bit off anything the
    /// host had already made executable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    pub content: FileContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<u32>, // UID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u32>, // GID
}

impl FileSpec {
    pub fn new(path: PathBuf, content: Vec<u8>, mode: Option<u32>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let sha256 = format!("{:x}", hasher.finalize());
        FileSpec {
            path,
            mode,
            content: FileContent::Content(content, sha256),
            owner: None,
            group: None,
        }
    }

    pub fn new_copy(src: PathBuf, dst: PathBuf, mode: Option<u32>) -> Self {
        let content = fs::read(src).expect("Failed to read path");
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let sha256 = format!("{:x}", hasher.finalize());
        FileSpec {
            path: dst,
            mode,
            content: FileContent::Content(content, sha256),
            owner: None,
            group: None,
        }
    }

    /// The change that puts this file on the host. Applying it writes the
    /// content *and* sets `mode`, so a spec that needs uploading never needs a
    /// separate [`WrongMode`] change too.
    fn missing_file(&self) -> MissingFile {
        MissingFile {
            path: self.path.clone(),
            content: self.content.clone(),
            owner: self.owner,
            group: self.group,
            mode: self.mode,
        }
    }
}

/// Parse a `mode="755"` property.
///
/// The value has to be a quoted octal string. A bare `755` is rejected rather
/// than silently misread: KDL parses it as the decimal number 755, which is a
/// perfectly valid — and completely different — mode (0o1363, setgid + rwx--x-wx).
fn parse_mode(entry: &KdlEntry, keyword: &str) -> u32 {
    let KdlValue::String(s) = entry.value() else {
        panic!(
            "{keyword}: mode {} is read as a decimal number — quote it as octal, e.g. mode=\"755\"",
            entry.value()
        );
    };
    u32::from_str_radix(s, 8)
        .unwrap_or_else(|_| panic!("{keyword}: mode \"{s}\" is not an octal number, e.g. mode=\"755\""))
}

/// Check if a path should be included based on include/exclude patterns.
/// Returns true if the path or any of its ancestors match an include pattern
/// and don't match an exclude pattern.
fn should_include_path(path: &Path, includes: &GlobSet, excludes: &GlobSet) -> bool {
    // Check if the file or any of its ancestor directories match exclude patterns
    for ancestor in path.ancestors() {
        if excludes.is_match(ancestor) {
            return false;
        }
    }

    // If there are no include patterns, include everything (that wasn't excluded)
    if includes.is_empty() {
        return true;
    }

    // Check if the file or any of its ancestor directories match include patterns
    for ancestor in path.ancestors() {
        if includes.is_match(ancestor) {
            return true;
        }
    }

    false
}

impl FromKdl for FileSpec {
    fn kdl_keywords() -> &'static [&'static str] {
        &["file", "cp"]
    }

    fn add_rules_to_state(state: &mut State, node: &KdlNode, context: &Context) {
        let keyword = node.name().value();
        // Paths are positional; `mode` is the only property either keyword takes.
        // Splitting them here means `cp a b mode="755"` and `cp mode="755" a b`
        // both work, instead of the property being consumed as a path.
        let mut args = Vec::new();
        let mut mode = None;
        for entry in node.entries() {
            match entry.name().map(|i| i.value()) {
                None => args.push(entry),
                Some("mode") => mode = Some(parse_mode(entry, keyword)),
                Some(z) => panic!("Unexpected option for {keyword}: {z}"),
            }
        }
        let mut args = args.into_iter();
        match keyword {
            "file" => {
                let dst = PathBuf::from(args.next().expect("file requires a path").expect_str());
                let file = FileSpec::new(dst, Vec::new(), mode);
                state.add_rule(file);
            }
            "cp" => {
                let src = context.local_path(args.next().expect("cp requires a source path").expect_str());
                let dst_str = args.next().expect("cp requires a destination path").expect_str();
                let mut dst = PathBuf::from(dst_str);
                let mut includes: GlobSetBuilder = GlobSetBuilder::new();
                let mut excludes: GlobSetBuilder = GlobSetBuilder::new();
                if let Some(child) = node.children() {
                    for n in child.nodes() {
                        match n.name().value() {
                            "include" => {
                                for e in n.entries() {
                                    let glob = format!("**/{}", e.expect_str().trim_end_matches("/"));
                                    let glob = Glob::new(&glob).expect("Invalid path for include directive");
                                    includes.add(glob);
                                }
                            }
                            "exclude" => {
                                for e in n.entries() {
                                    let glob = format!("**/{}", e.expect_str().trim_end_matches("/"));
                                    let glob = Glob::new(&glob).expect("Invalid path for exclude directive");
                                    excludes.add(glob);
                                }
                            }
                            _ => panic!("Unexpected directive for cp: {}", n.name().value()),
                        }
                    }
                }
                let includes = includes.build().expect("Failed to build includes");
                let excludes = excludes.build().expect("Failed to build excludes");
                if src.is_dir() {
                    let entries = walkdir::WalkDir::new(&src)
                        .into_iter()
                        .flat_map(|e| e.ok())
                        .filter(|e| e.path().is_file());
                    let mut files = Vec::new();
                    for entry in entries {
                        let entry = entry.path();
                        let relative_path = entry.strip_prefix(&src).expect("Path should be under src");
                        if !should_include_path(relative_path, &includes, &excludes) {
                            continue;
                        }
                        let target_path = dst.join(relative_path);
                        files.push(FileSpec::new_copy(entry.to_path_buf(), target_path, mode));
                    } // walk the dir recursively. collect every included file into one fileset
                    state.add_rule(FileSetSpec { root: dst, files });
                } else {
                    if dst_str.ends_with('/') {
                        dst.push(src.file_name().expect("Must have a file name."));
                    }
                    let file = FileSpec::new_copy(src, dst, mode);
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

    fn kind(&self) -> &'static str {
        "file"
    }

    fn identifier(&self) -> &str {
        self.path.to_str().unwrap()
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl RuleOverSsh for FileSpec {
    async fn check_ssh(&self, session: &openssh::Session) -> Result<Vec<Box<dyn Modification>>, Error> {
        let path = self.path.to_str().ok_or("file path is not valid utf-8")?;
        let needs_upload = match &self.content {
            FileContent::Content(_, sha256) => {
                let output = session.command("sha256sum").arg(path).output().await?;
                let output = String::from_utf8_lossy(&output.stdout);
                let remote_hash = output.split_whitespace().next().unwrap_or_default();
                sha256 != remote_hash
            }
            FileContent::Url(_) => !session.command("test").arg("-f").arg(path).output().await?.status.success(),
        };

        // An upload sets the mode on its way out, so it subsumes a mode change.
        if needs_upload {
            return Ok(vec![Box::new(FileChange::MissingFile(self.missing_file()))]);
        }

        // The content is already right, but the mode may have drifted. Only
        // worth a round-trip when there's a mode to enforce; the file is known
        // to exist here, so `stat` failing means something else is wrong.
        let Some(mode) = self.mode else {
            return Ok(Vec::new());
        };
        let output = session.command("stat").arg("-c").arg("%a").arg(path).output().await?;
        let remote = String::from_utf8_lossy(&output.stdout);
        let remote = u32::from_str_radix(remote.trim(), 8)
            .map_err(|_| format!("could not read the mode of {path}: stat printed {remote:?}"))?;
        if remote == mode {
            return Ok(Vec::new());
        }
        Ok(vec![Box::new(FileChange::WrongMode(WrongMode {
            path: self.path.clone(),
            mode,
        }))])
    }
}

/// A whole-directory copy. Instead of checking each file with its own `sha256sum`
/// command over SSH (one round-trip per file), the fileset is verified with a
/// single command that hashes every file under `root` at once. Only the files
/// whose hashes differ (or are missing) are uploaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSetSpec {
    /// Remote destination root. Used to enumerate existing remote files in one shot.
    pub root: PathBuf,
    /// Every included file, with its absolute target `path`, content and hash.
    pub files: Vec<FileSpec>,
}

impl Rule for FileSetSpec {
    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn crate::RuleOverSsh> {
        Some(self)
    }

    fn check(&self) -> Result<Vec<Box<dyn Modification>>, Error> {
        todo!()
    }

    fn kind(&self) -> &'static str {
        "file"
    }

    fn identifier(&self) -> &str {
        self.root.to_str().unwrap()
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl RuleOverSsh for FileSetSpec {
    async fn check_ssh(&self, session: &openssh::Session) -> Result<Vec<Box<dyn Modification>>, Error> {
        // Hash every existing file under `root` in a single round-trip. `find`
        // errors (e.g. missing root) are swallowed so it degrades to "no remote
        // files", which makes every local file appear missing and get uploaded.
        let root = self.root.to_str().ok_or("root path is not valid utf-8")?;
        let script = format!(
            "find {} -type f -print0 2>/dev/null | xargs -0 sha256sum 2>/dev/null",
            sh_single_quote(root)
        );
        let output = session.command("sh").arg("-c").arg(&script).output().await?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse `sha256sum` output: "<64-hex-hash>  <path>" per line. The path can
        // contain spaces, so split on the two-space separator only.
        let mut remote: HashMap<&str, &str> = HashMap::new();
        for line in stdout.lines() {
            if let Some((hash, path)) = line.split_once("  ") {
                remote.insert(path, hash);
            }
        }

        // Modes, if any are being enforced: a second whole-tree walk rather
        // than a `stat` per file, for the same reason the hashes are one call.
        // The `%a  %n` separator mirrors sha256sum's, so paths with spaces
        // survive the same way.
        let mut remote_modes: HashMap<String, u32> = HashMap::new();
        if self.files.iter().any(|f| f.mode.is_some()) {
            let script = format!(
                "find {} -type f -exec stat -c '%a  %n' {{}} + 2>/dev/null",
                sh_single_quote(root)
            );
            let output = session.command("sh").arg("-c").arg(&script).output().await?;
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Some((mode, path)) = line.split_once("  ")
                    && let Ok(mode) = u32::from_str_radix(mode.trim(), 8)
                {
                    remote_modes.insert(path.to_string(), mode);
                }
            }
        }

        let mut changes: Vec<Box<dyn Modification>> = Vec::new();
        for file in &self.files {
            let path_str = file.path.to_str().ok_or("file path is not valid utf-8")?;
            let needs_change = match &file.content {
                FileContent::Content(_, sha256) => remote.get(path_str) != Some(&sha256.as_str()),
                FileContent::Url(_) => !remote.contains_key(path_str),
            };
            if needs_change {
                // The upload carries the mode with it.
                changes.push(Box::new(FileChange::MissingFile(file.missing_file())));
            } else if let Some(mode) = file.mode
                && remote_modes.get(path_str) != Some(&mode)
            {
                changes.push(Box::new(FileChange::WrongMode(WrongMode {
                    path: file.path.clone(),
                    mode,
                })));
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
    mode: Option<u32>,
}

/// The file is already correct, but its permission bits are not.
#[derive(Debug, Serialize)]
pub struct WrongMode {
    path: PathBuf,
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
    WrongMode(WrongMode),
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
            FileChange::WrongMode { .. } => todo!(),
            // FileChange::MissingDirectory { .. } => todo!(),
            // FileChange::MissingSymlink { .. } => todo!(),
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
                // sftp creates at its own default and leaves an existing file's
                // mode alone, so anything that has to be executable (or has to
                // not be world-readable) is set here, after the bytes land.
                if let Some(mode) = file.mode {
                    chmod(&session, &file.path, mode).await?;
                }
            }
            FileChange::WrongMode(change) => chmod(&session, &change.path, change.mode).await?,
            // FileChange::MissingDirectory { .. } => todo!(),
            // FileChange::MissingSymlink { .. } => todo!(),
        }
        Ok(())
    }
}

#[cfg(feature = "ssh")]
async fn chmod(session: &openssh::Session, path: &Path, mode: u32) -> Result<(), Error> {
    let path = path.to_str().ok_or("file path is not valid utf-8")?;
    let output = session
        .command("chmod")
        .arg(format!("{mode:o}"))
        .arg(path)
        .output()
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("chmod {mode:o} {path} failed: {}", stderr.trim()).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `mode` off the first node of a one-line KDL document.
    fn mode_of(kdl: &str) -> u32 {
        let doc = kdl::KdlDocument::parse(kdl).expect("test kdl should parse");
        let node = doc.nodes().first().expect("one node");
        let entry = node
            .entries()
            .iter()
            .find(|e| e.name().is_some_and(|n| n.value() == "mode"))
            .expect("a mode property");
        parse_mode(entry, node.name().value())
    }

    #[test]
    fn mode_is_parsed_as_octal() {
        assert_eq!(mode_of(r#"cp a b mode="755""#), 0o755);
        assert_eq!(mode_of(r#"cp a b mode="0755""#), 0o755);
        assert_eq!(mode_of(r#"file a mode="600""#), 0o600);
        assert_eq!(mode_of(r#"file a mode="4755""#), 0o4755);
    }

    /// KDL reads a bare `755` as decimal 755 (0o1363). Taking that at face
    /// value would quietly install a setgid file, so it has to be rejected.
    #[test]
    #[should_panic(expected = "quote it as octal")]
    fn unquoted_mode_is_rejected() {
        mode_of("cp a b mode=755");
    }

    #[test]
    #[should_panic(expected = "not an octal number")]
    fn non_octal_mode_is_rejected() {
        mode_of(r#"cp a b mode="799""#);
    }

    #[test]
    fn test_include_matches_directory_and_children() {
        let includes = GlobSetBuilder::new()
            .add(Glob::new("**/build").unwrap())
            .add(Glob::new("**/dist").unwrap())
            .add(Glob::new("**/run.sh").unwrap())
            .add(Glob::new("**/conf.yaml").unwrap())
            .build()
            .unwrap();
        let excludes = GlobSetBuilder::new().build().unwrap();

        // Should match the directory itself
        assert!(should_include_path(
            Path::new("foo/bar/build/bar.xml"),
            &includes,
            &excludes
        ));

        // Should match children of the directory
        assert!(should_include_path(
            Path::new("foo/bar/build/output.txt"),
            &includes,
            &excludes
        ));
        assert!(should_include_path(
            Path::new("foo/bar/build/nested/file.txt"),
            &includes,
            &excludes
        ));

        // Should not match unrelated paths
        assert!(!should_include_path(
            Path::new("foo/bar/src/main.rs"),
            &includes,
            &excludes
        ));
    }

    #[test]
    fn test_include_matches_files_directly() {
        let includes = GlobSetBuilder::new()
            .add(Glob::new("**/build.rs").unwrap())
            .build()
            .unwrap();
        let excludes = GlobSetBuilder::new().build().unwrap();

        // Should match the file itself
        assert!(should_include_path(Path::new("foo/bar/build.rs"), &includes, &excludes));
        assert!(should_include_path(Path::new("build.rs"), &includes, &excludes));

        // Should not match non-matching files
        assert!(!should_include_path(Path::new("foo/bar/main.rs"), &includes, &excludes));
    }
}
