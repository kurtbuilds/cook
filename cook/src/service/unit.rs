//! Reading values back out of systemd unit files.
//!
//! cook takes unit files as opaque content to install, but a few directives
//! describe preconditions the host has to satisfy before the unit can start —
//! `WorkingDirectory=` being the first of them. This module is the minimal
//! amount of unit-file interpretation needed to enforce those preconditions.
//!
//! The parsing follows systemd's file format where it matters and stops short
//! where it doesn't: sections, comments, line continuations, quoting and the
//! `-` "ignore failure" prefix are handled; specifier expansion (`%h`, `%S`,
//! …) is not, because those resolve against the *remote* host's runtime
//! context, which cook cannot know at plan time.

use serde::{Deserialize, Serialize};

/// Split a unit file into logical lines, joining any line continued with a
/// trailing `\` onto the one that follows it.
fn logical_lines(content: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut pending = String::new();
    for raw in content.lines() {
        let line = raw.trim();
        if let Some(head) = line.strip_suffix('\\') {
            pending.push_str(head.trim_end());
            pending.push(' ');
            continue;
        }
        pending.push_str(line);
        lines.push(std::mem::take(&mut pending));
    }
    // A file whose last line ends in `\` leaves the continuation dangling.
    if !pending.is_empty() {
        lines.push(pending);
    }
    lines
}

/// The raw value of `key` in `section` of a unit file, or `None` if absent.
///
/// Later assignments win, matching systemd: a unit that sets a key twice takes
/// the last value. An explicitly empty assignment (`Key=`) resets the value to
/// `None`, which is how systemd clears a directive inherited from a drop-in.
fn directive(content: &str, section: &str, key: &str) -> Option<String> {
    let mut current_section: Option<String> = None;
    let mut value = None;

    for line in logical_lines(content) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current_section = Some(name.trim().to_string());
            continue;
        }
        if current_section.as_deref() != Some(section) {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        let v = v.trim();
        value = (!v.is_empty()).then(|| v.to_string());
    }

    value
}

/// Strip one layer of matching surrounding quotes, as systemd does when it
/// reads a directive value.
fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(quote).and_then(|v| v.strip_suffix(quote)) {
            return inner;
        }
    }
    value
}

/// Interpret a directive value as a systemd boolean.
fn boolean(content: &str, section: &str, key: &str) -> Option<bool> {
    let value = directive(content, section, key)?;
    match unquote(value.trim()).trim() {
        "1" | "yes" | "true" | "on" => Some(true),
        "0" | "no" | "false" | "off" => Some(false),
        _ => None,
    }
}

/// The `WorkingDirectory=` of a `.service` unit, as written in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingDirectory {
    /// The path, with the `-` prefix and any quoting removed. May be the
    /// literal `~` (the service user's home) or contain `%` specifiers.
    pub path: String,
    /// Set by the `-` prefix: systemd starts the unit even when the directory
    /// is missing, so the author has declared its existence optional.
    pub optional: bool,
}

/// The `WorkingDirectory=` of a `.service` unit, as written in the file.
///
/// Use [`required_working_directory`] when the directory needs to be acted on;
/// this accessor reports the directive verbatim, optional paths included.
pub fn working_directory(content: &str) -> Option<WorkingDirectory> {
    let value = directive(content, "Service", "WorkingDirectory")?;
    let (value, optional) = match value.strip_prefix('-') {
        Some(rest) => (rest.trim(), true),
        None => (value.as_str(), false),
    };
    let path = unquote(value).trim().to_string();
    (!path.is_empty()).then_some(WorkingDirectory { path, optional })
}

/// The user and group a `.service` unit's processes run as, when cook can
/// resolve them.
///
/// `group` is `None` when the unit sets only `User=`; systemd then runs the
/// process under that user's login group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceOwner {
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// The `User=`/`Group=` a unit's processes run as, restricted to names cook can
/// act on.
///
/// Returns `None` when the unit runs as the service manager's own user (no
/// `User=`), when either name contains a `%` specifier, or when
/// `DynamicUser=yes` — systemd allocates that user at start time, so it does
/// not exist for cook to chown to.
pub fn service_owner(content: &str) -> Option<ServiceOwner> {
    if boolean(content, "Service", "DynamicUser") == Some(true) {
        return None;
    }
    let user = unquote(directive(content, "Service", "User")?.trim())
        .trim()
        .to_string();
    let group = directive(content, "Service", "Group").map(|g| unquote(g.trim()).trim().to_string());
    if user.is_empty() || user.contains('%') || group.as_deref().is_some_and(|g| g.contains('%')) {
        return None;
    }
    Some(ServiceOwner {
        user,
        group: group.filter(|g| !g.is_empty()),
    })
}

/// A directory that must exist on the target before the unit can start, plus
/// the ownership the unit's processes need on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredWorkingDirectory {
    pub path: String,
    /// `None` when the unit runs as the service manager's own user, or when the
    /// owner cannot be resolved ahead of time — cook then leaves ownership as
    /// whatever creating the directory produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<ServiceOwner>,
}

/// The working directory cook must ensure exists, or `None` if there is nothing
/// to enforce.
///
/// Yields `None` when the unit sets no `WorkingDirectory=`, when the path is
/// prefixed with `-` (the author has declared a missing directory acceptable,
/// so cook does not create one), or when the path's meaning depends on the
/// remote host's runtime context: `~` resolves to the service user's home and
/// `%` specifiers resolve against systemd's runtime state, neither of which
/// cook can evaluate at plan time.
pub fn required_working_directory(content: &str) -> Option<RequiredWorkingDirectory> {
    let WorkingDirectory { path, optional } = working_directory(content)?;
    if optional {
        tracing::debug!(
            working_directory = %path,
            "WorkingDirectory is prefixed with `-`; systemd starts the unit without it, so cook will not create it"
        );
        return None;
    }
    if path == "~" || path.contains('%') {
        tracing::warn!(
            working_directory = %path,
            "WorkingDirectory resolves against the remote host's runtime context; cook will not create it"
        );
        return None;
    }
    Some(RequiredWorkingDirectory {
        owner: service_owner(content),
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::{required_working_directory, service_owner, working_directory};

    const UNIT: &str = "\
[Unit]
Description=Example
WorkingDirectory=/wrong/section

[Service]
ExecStart=/usr/bin/example
WorkingDirectory=/srv/example
User=example
Group=example-grp
Restart=always

[Install]
WantedBy=multi-user.target
";

    /// The path of the parsed `WorkingDirectory=`, ignoring the `-` flag.
    fn path(content: &str) -> Option<String> {
        working_directory(content).map(|w| w.path)
    }

    #[test]
    fn reads_working_directory_from_the_service_section() {
        assert_eq!(path(UNIT).as_deref(), Some("/srv/example"));
    }

    #[test]
    fn absent_directive_is_none() {
        let unit = "[Service]\nExecStart=/usr/bin/example\n";
        assert_eq!(path(unit), None);
    }

    #[test]
    fn ignores_comments_and_tolerates_indentation() {
        let unit = "[Service]\n# WorkingDirectory=/commented\n; WorkingDirectory=/nope\n  WorkingDirectory=/srv/app\n";
        assert_eq!(path(unit).as_deref(), Some("/srv/app"));
    }

    #[test]
    fn strips_the_ignore_failure_prefix_and_quotes() {
        let dir = working_directory("[Service]\nWorkingDirectory=-\"/srv/quoted dir\"\n").expect("parsed");
        assert_eq!(dir.path, "/srv/quoted dir");
        assert!(dir.optional);

        let dir = working_directory(UNIT).expect("parsed");
        assert!(!dir.optional);
    }

    #[test]
    fn last_assignment_wins_and_an_empty_one_resets() {
        let repeated = "[Service]\nWorkingDirectory=/first\nWorkingDirectory=/second\n";
        assert_eq!(path(repeated).as_deref(), Some("/second"));

        let reset = "[Service]\nWorkingDirectory=/first\nWorkingDirectory=\n";
        assert_eq!(path(reset), None);
    }

    #[test]
    fn joins_continuation_lines() {
        let unit = "[Service]\nWorkingDirectory=\\\n  /srv/continued\n";
        assert_eq!(path(unit).as_deref(), Some("/srv/continued"));
    }

    #[test]
    fn host_resolved_paths_are_not_required() {
        assert_eq!(required_working_directory("[Service]\nWorkingDirectory=~\n"), None);
        assert_eq!(required_working_directory("[Service]\nWorkingDirectory=%h/app\n"), None);
    }

    #[test]
    fn an_optional_directory_is_not_required() {
        // The `-` prefix means systemd starts the unit even when the directory
        // is missing, so its author has opted out of cook creating it.
        assert_eq!(
            required_working_directory("[Service]\nWorkingDirectory=-/srv/app\n"),
            None
        );

        let required = required_working_directory("[Service]\nWorkingDirectory=/srv/app\n").expect("required");
        assert_eq!(required.path, "/srv/app");
    }

    #[test]
    fn required_directory_carries_the_units_owner() {
        let required = required_working_directory(UNIT).expect("required");
        assert_eq!(required.path, "/srv/example");
        let owner = required.owner.expect("owner");
        assert_eq!(owner.user, "example");
        assert_eq!(owner.group.as_deref(), Some("example-grp"));
    }

    #[test]
    fn group_is_optional_and_defaults_to_the_users_login_group() {
        let owner = service_owner("[Service]\nUser=example\n").expect("owner");
        assert_eq!(owner.user, "example");
        assert_eq!(owner.group, None);
    }

    #[test]
    fn units_without_a_user_run_as_the_manager_and_need_no_chown() {
        assert_eq!(service_owner("[Service]\nExecStart=/usr/bin/example\n"), None);
    }

    #[test]
    fn owners_cook_cannot_resolve_are_skipped() {
        // Allocated by systemd at start time, so the account does not exist yet.
        assert_eq!(service_owner("[Service]\nUser=example\nDynamicUser=yes\n"), None);
        // Specifiers resolve against the remote's runtime state.
        assert_eq!(service_owner("[Service]\nUser=%i\n"), None);
        assert_eq!(service_owner("[Service]\nUser=example\nGroup=%i\n"), None);
        // An explicit `DynamicUser=no` is not an obstacle.
        let owner = service_owner("[Service]\nUser=example\nDynamicUser=no\n").expect("owner");
        assert_eq!(owner.user, "example");
    }
}
