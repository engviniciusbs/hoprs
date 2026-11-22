//! The thirteen tools an agent sees: name, title, description, input schema, the level a
//! call needs and the protocol's behaviour hints.
//!
//! This is plain data. The relay serves it unchanged whether or not the app is running
//! (so `tools/list` never depends on a connection), and the service uses [`Need`] to
//! decide what a call may do. The level of each tool is a security contract of the
//! design (section "Permissions"): the test `levels_are_pinned` names every one.
//!
//! The descriptions are what a model reads to decide how to use hoprs. They are English
//! only, say what the tool does, what each argument means, the limits that apply and
//! what comes back, and promise nothing the design does not.

use serde_json::{Value, json};

/// The level of access a call needs on its site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// No site argument: the call names no site.
    Nothing,
    /// The site allows reading (or everything).
    Read,
    /// The site allows everything.
    All,
    /// The site allows everything and is an SSH site.
    AllSsh,
}

/// One tool of the table.
// TODO(Task 13): remove
#[allow(dead_code)]
pub struct ToolSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// The input schema: `{"type":"object","properties":…,"required":…,"additionalProperties":false}`.
    pub schema: fn() -> Value,
    pub need: Need,
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
}

/// The thirteen tools, in the order `tools/list` serves them.
// TODO(Task 13): remove
#[allow(dead_code)]
pub fn all() -> &'static [ToolSpec] {
    &TOOLS
}

/// The tool called `name`.
pub fn find(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|t| t.name == name)
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn site() -> Value {
    text("The site's id, as list_sites returns it.")
}

fn text(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn flag(description: &str) -> Value {
    json!({"type": "boolean", "description": description})
}

fn schema_list_sites() -> Value {
    object(json!({}), &[])
}

fn schema_list_directory() -> Value {
    object(
        json!({
            "site": site(),
            "path": text("Absolute path of the folder on the server, for example /var/www."),
            "offset": {"type": "integer", "minimum": 0, "description":
                "Entries to skip, to read the next page of a long folder. Default 0."},
            "limit": {"type": "integer", "minimum": 1, "maximum": 5000, "description":
                "Entries to return, 1 to 5000. Default 1000."},
        }),
        &["site", "path"],
    )
}

fn schema_stat() -> Value {
    object(
        json!({
            "site": site(),
            "path": text("Absolute path of the file, folder or link on the server."),
        }),
        &["site", "path"],
    )
}

fn schema_read_file() -> Value {
    object(
        json!({
            "site": site(),
            "path": text("Absolute path of the file on the server."),
            "offset": {"type": "integer", "minimum": 0, "description":
                "Byte to start reading at. Default 0."},
            "length": {"type": "integer", "minimum": 0, "description":
                "Number of bytes to read, at most 1 MiB (1048576). Default: to the end of the file."},
        }),
        &["site", "path"],
    )
}

fn schema_download() -> Value {
    object(
        json!({
            "site": site(),
            "remote_path": text("Absolute path on the server of the file or folder to download."),
            "local_path": text("Absolute path on this computer that the download will have: the file or folder itself, not the folder to put it in."),
            "overwrite": flag("Replace what is already at local_path. Default false: an existing file is skipped, not replaced."),
        }),
        &["site", "remote_path", "local_path"],
    )
}

fn schema_transfer_status() -> Value {
    object(
        json!({
            "transfer": {"type": "integer", "minimum": 0, "description":
                "The transfer number that download or upload answered."},
        }),
        &["transfer"],
    )
}

fn schema_write_file() -> Value {
    object(
        json!({
            "site": site(),
            "path": text("Absolute path of the file to write on the server."),
            "content": text("The whole new content of the file, as text (UTF-8), at most 1 MiB (1048576 bytes)."),
            "overwrite": flag("Replace the file if it exists. Default false: an existing file is an error and nothing is written."),
        }),
        &["site", "path", "content"],
    )
}

fn schema_make_directory() -> Value {
    object(
        json!({
            "site": site(),
            "path": text("Absolute path of the folder to create on the server."),
        }),
        &["site", "path"],
    )
}

fn schema_rename() -> Value {
    object(
        json!({
            "site": site(),
            "from": text("Absolute path of the file or folder on the server."),
            "to": text("Absolute path it will have afterwards (a different folder moves it)."),
        }),
        &["site", "from", "to"],
    )
}

fn schema_set_permissions() -> Value {
    object(
        json!({
            "site": site(),
            "path": text("Absolute path of the file or folder on the server."),
            "mode": {"type": "string", "pattern": "^[0-7]{3,4}$", "description":
                "The new permissions as 3 or 4 octal digits, for example \"644\" or \"0755\"."},
        }),
        &["site", "path", "mode"],
    )
}

fn schema_upload() -> Value {
    object(
        json!({
            "site": site(),
            "local_path": text("Absolute path on this computer of the file or folder to upload."),
            "remote_path": text("Absolute path on the server that the upload will have: the file or folder itself, not the folder to put it in."),
            "overwrite": flag("Replace what is already at remote_path. Default false: an existing file is skipped, not replaced."),
        }),
        &["site", "local_path", "remote_path"],
    )
}

fn schema_delete() -> Value {
    object(
        json!({
            "site": site(),
            "path": text("Absolute path of the file or folder to delete on the server."),
            "recursive": flag("Delete a folder and everything in it. Default false: a folder that is not empty is an error."),
        }),
        &["site", "path"],
    )
}

fn schema_run_command() -> Value {
    object(
        json!({
            "site": site(),
            "command": text("The command line, run by the server's shell."),
            "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 600, "description":
                "Seconds the command may run before it is stopped, 1 to 600. Default 60."},
        }),
        &["site", "command"],
    )
}

static TOOLS: [ToolSpec; 13] = [
    ToolSpec {
        name: "list_sites",
        title: "List sites",
        description: "Lists the sites the user has opened to agents in hoprs, each with its id, name, protocol (ssh, ftp, ftps-explicit or ftps-implicit), host, port, user, the access level you have on it (\"read\" or \"all\"), whether hoprs is connected to it now, and its start folder. Call this first: every other tool that takes a site wants the id from here. Sites the user has not opened to agents are not listed. Needs no access to any site.",
        schema: schema_list_sites,
        need: Need::Nothing,
        read_only: true,
        destructive: false,
        idempotent: false,
        open_world: false,
    },
    ToolSpec {
        name: "list_directory",
        title: "List a folder",
        description: "Lists the entries of a folder on a site: name, kind (file, dir or symlink), size, modification time and permissions, sorted by name. Returns at most 1000 entries by default and at most 5000 per call (limit); when the folder has more, the answer says truncated and gives the total, and you read the next page by calling again with offset. Needs read access. The site is an id from list_sites.",
        schema: schema_list_directory,
        need: Need::Read,
        read_only: true,
        destructive: false,
        idempotent: false,
        open_world: true,
    },
    ToolSpec {
        name: "stat",
        title: "Describe a path",
        description: "Describes one path on a site: its kind (file, dir or symlink), size, modification time and permissions. An error says the path does not exist. Needs read access. The site is an id from list_sites.",
        schema: schema_stat,
        need: Need::Read,
        read_only: true,
        destructive: false,
        idempotent: false,
        open_world: true,
    },
    ToolSpec {
        name: "read_file",
        title: "Read a text file",
        description: "Reads text from a file on a site and returns it as text (UTF-8). At most 1 MiB (1048576 bytes) per call: for a longer file read it in pieces with offset and length, or use download. The answer starts with a header (size of the file, offset, length returned, next_offset and whether the end was reached) followed by the text. A file that is not valid UTF-8 cannot be read this way: use download. Needs read access. The site is an id from list_sites.",
        schema: schema_read_file,
        need: Need::Read,
        read_only: true,
        destructive: false,
        idempotent: false,
        open_world: true,
    },
    ToolSpec {
        name: "download",
        title: "Download to this computer",
        description: "Downloads a file or a whole folder from a site to a path on this computer (the one hoprs runs on). local_path is the path the download will have, not the folder to put it in. With overwrite false (the default) a file that already exists there is skipped, not replaced; with true it is replaced, and that destroys the old file. hoprs's own files, ~/.ssh and the key files of saved sites are protected and refused. A transfer that finishes within about 60 seconds is answered with its result; a longer one is answered with state \"running\" and a transfer number, and you follow it with transfer_status. Needs read access. The site is an id from list_sites.",
        schema: schema_download,
        need: Need::Read,
        read_only: false,
        destructive: true,
        idempotent: true,
        open_world: true,
    },
    ToolSpec {
        name: "transfer_status",
        title: "Check a transfer",
        description: "Tells how a download or upload that you started is going: its state (\"running\", \"paused\", \"done\", \"skipped\", \"failed\" or \"cancelled\"), the files and bytes done, and the files that failed with the reason. Pass the transfer number that download or upload answered; a transfer that is still \"running\" can be checked again later. Only transfers started by agents are known, and only while you still have read access to their site. Needs no site argument; the sites come from list_sites.",
        schema: schema_transfer_status,
        need: Need::Nothing,
        read_only: true,
        destructive: false,
        idempotent: false,
        open_world: false,
    },
    ToolSpec {
        name: "write_file",
        title: "Write a text file",
        description: "Writes a text file on a site: the whole content replaces the file. At most 1 MiB (1048576 bytes) of content per call; for more, use upload. With overwrite false (the default) a path that already exists is an error and nothing is written; with true the old content is destroyed. A folder or a symbolic link at the path is an error (as the server reports the path: some FTP servers report a link as the file it points to). Over SFTP the file is written to a temporary name and renamed over the old one; over FTP and FTPS it is written in place, so a write cut half way can leave the file incomplete (the error says so). Needs full access. The site is an id from list_sites.",
        schema: schema_write_file,
        need: Need::All,
        read_only: false,
        destructive: true,
        idempotent: true,
        open_world: true,
    },
    ToolSpec {
        name: "make_directory",
        title: "Create a folder",
        description: "Creates a folder on a site. The parent folder must exist. Creating a folder that already exists is not an error. Needs full access. The site is an id from list_sites.",
        schema: schema_make_directory,
        need: Need::All,
        read_only: false,
        destructive: false,
        idempotent: true,
        open_world: true,
    },
    ToolSpec {
        name: "rename",
        title: "Rename or move",
        description: "Renames or moves a file or folder on a site (a different folder in `to` moves it). An existing target is never replaced: if something is already at `to`, the call is an error and nothing changes. Needs full access. The site is an id from list_sites.",
        schema: schema_rename,
        need: Need::All,
        read_only: false,
        destructive: false,
        idempotent: false,
        open_world: true,
    },
    ToolSpec {
        name: "set_permissions",
        title: "Change permissions",
        description: "Changes the permissions of a file or folder on a site, given as 3 or 4 octal digits (for example 644). A symbolic link at the path is an error (as the server reports the path: some FTP servers report a link as what it points to). Not every server supports it (FTP servers often do not). Needs full access. The site is an id from list_sites.",
        schema: schema_set_permissions,
        need: Need::All,
        read_only: false,
        destructive: false,
        idempotent: true,
        open_world: true,
    },
    ToolSpec {
        name: "upload",
        title: "Upload from this computer",
        description: "Uploads a file or a whole folder from this computer (the one hoprs runs on) to a site. remote_path is the path the upload will have, not the folder to put it in. With overwrite false (the default) a file that already exists there is skipped, not replaced; with true it is replaced, and that destroys the old file. hoprs's own files, ~/.ssh and the key files of saved sites are protected and refused. A transfer that finishes within about 60 seconds is answered with its result; a longer one is answered with state \"running\" and a transfer number, and you follow it with transfer_status. Needs full access. The site is an id from list_sites.",
        schema: schema_upload,
        need: Need::All,
        read_only: false,
        destructive: true,
        idempotent: true,
        open_world: true,
    },
    ToolSpec {
        name: "delete",
        title: "Delete",
        description: "Deletes a file, a symbolic link or a folder on a site. This cannot be undone. A folder that is not empty is an error unless recursive is true, which deletes everything inside it. Over SFTP a link is removed as a link, never what it points to; some FTP servers report a link to a folder as a folder, and a recursive delete then deletes that folder's contents. The root folder / is refused. The answer says how many items were deleted. Needs full access. The site is an id from list_sites.",
        schema: schema_delete,
        need: Need::All,
        read_only: false,
        destructive: true,
        idempotent: true,
        open_world: true,
    },
    ToolSpec {
        name: "run_command",
        title: "Run a command over SSH",
        description: "Runs one command on an SSH site and returns its exit code (or the signal that ended it), whether it timed out, and its output (stdout, then stderr). There is no terminal and no input: the command cannot ask a question, and programs that need a terminal will not work. After timeout_seconds (60 by default, 600 at most) hoprs stops waiting and sends the command a KILL signal; the answer then says timed_out, and the process may still be running on the server. If the connection is lost before the command ends, the answer says lost. Each output stream is cut at 256 KiB and the answer says when it was. The command can change anything the site's user can. One command at a time runs per site; the next waits. Only for SSH sites: FTP sites cannot run commands. Needs full access. The site is an id from list_sites.",
        schema: schema_run_command,
        need: Need::AllSsh,
        read_only: false,
        destructive: true,
        idempotent: false,
        open_world: true,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const NAMES: [&str; 13] = [
        "list_sites",
        "list_directory",
        "stat",
        "read_file",
        "download",
        "transfer_status",
        "write_file",
        "make_directory",
        "rename",
        "set_permissions",
        "upload",
        "delete",
        "run_command",
    ];

    #[test]
    fn thirteen_tools_in_order() {
        let names: Vec<&str> = all().iter().map(|t| t.name).collect();
        assert_eq!(names, NAMES);
        for name in NAMES {
            assert_eq!(find(name).map(|t| t.name), Some(name));
        }
        assert!(find("shell").is_none());
        assert!(find("").is_none());
    }

    #[test]
    fn titles_match_the_table() {
        let titles: Vec<&str> = all().iter().map(|t| t.title).collect();
        assert_eq!(
            titles,
            [
                "List sites",
                "List a folder",
                "Describe a path",
                "Read a text file",
                "Download to this computer",
                "Check a transfer",
                "Write a text file",
                "Create a folder",
                "Rename or move",
                "Change permissions",
                "Upload from this computer",
                "Delete",
                "Run a command over SSH",
            ]
        );
    }

    #[test]
    fn names_are_valid_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for tool in all() {
            assert!((1..=128).contains(&tool.name.len()), "{}", tool.name);
            assert!(
                tool.name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')),
                "{}",
                tool.name
            );
            assert!(seen.insert(tool.name), "duplicate {}", tool.name);
            assert!(!tool.title.is_empty() && !tool.description.is_empty());
        }
    }

    /// `(tool, required arguments, all arguments)`, from the plan's ruling 6.
    const ARGS: [(&str, &[&str], &[&str]); 13] = [
        ("list_sites", &[], &[]),
        (
            "list_directory",
            &["site", "path"],
            &["site", "path", "offset", "limit"],
        ),
        ("stat", &["site", "path"], &["site", "path"]),
        (
            "read_file",
            &["site", "path"],
            &["site", "path", "offset", "length"],
        ),
        (
            "download",
            &["site", "remote_path", "local_path"],
            &["site", "remote_path", "local_path", "overwrite"],
        ),
        ("transfer_status", &["transfer"], &["transfer"]),
        (
            "write_file",
            &["site", "path", "content"],
            &["site", "path", "content", "overwrite"],
        ),
        ("make_directory", &["site", "path"], &["site", "path"]),
        ("rename", &["site", "from", "to"], &["site", "from", "to"]),
        (
            "set_permissions",
            &["site", "path", "mode"],
            &["site", "path", "mode"],
        ),
        (
            "upload",
            &["site", "local_path", "remote_path"],
            &["site", "local_path", "remote_path", "overwrite"],
        ),
        ("delete", &["site", "path"], &["site", "path", "recursive"]),
        (
            "run_command",
            &["site", "command"],
            &["site", "command", "timeout_seconds"],
        ),
    ];

    #[test]
    fn every_schema_is_a_closed_object() {
        for tool in all() {
            let schema = (tool.schema)();
            // It must survive a trip through text, as the relay serves it.
            let schema: Value = serde_json::from_str(&schema.to_string()).unwrap();
            assert_eq!(schema["type"], "object", "{}", tool.name);
            assert_eq!(schema["additionalProperties"], false, "{}", tool.name);
            let props = schema["properties"].as_object().unwrap();
            let required: Vec<&str> = schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            for name in &required {
                assert!(props.contains_key(*name), "{} {name}", tool.name);
            }
            for (name, prop) in props {
                let text = prop["description"].as_str().unwrap_or("");
                assert!(!text.is_empty(), "{}.{name} has no description", tool.name);
                assert!(prop["type"].is_string(), "{}.{name} has no type", tool.name);
            }
            let (_, want_required, want_all) =
                ARGS.iter().find(|(n, _, _)| *n == tool.name).unwrap();
            assert_eq!(required, *want_required, "{}", tool.name);
            let mut have: Vec<&str> = props.keys().map(String::as_str).collect();
            let mut want: Vec<&str> = want_all.to_vec();
            have.sort_unstable();
            want.sort_unstable();
            assert_eq!(have, want, "{}", tool.name);
        }
    }

    #[test]
    fn argument_types_and_bounds() {
        let schema = |name: &str| (find(name).unwrap().schema)();
        let list = schema("list_directory");
        assert_eq!(list["properties"]["limit"]["minimum"], 1);
        assert_eq!(list["properties"]["limit"]["maximum"], 5000);
        assert_eq!(list["properties"]["offset"]["minimum"], 0);
        assert_eq!(list["properties"]["limit"]["type"], "integer");
        let run = schema("run_command");
        assert_eq!(run["properties"]["timeout_seconds"]["minimum"], 1);
        assert_eq!(run["properties"]["timeout_seconds"]["maximum"], 600);
        assert_eq!(
            schema("transfer_status")["properties"]["transfer"]["type"],
            "integer"
        );
        assert_eq!(
            schema("download")["properties"]["overwrite"]["type"],
            "boolean"
        );
        assert_eq!(
            schema("delete")["properties"]["recursive"]["type"],
            "boolean"
        );
        assert_eq!(
            schema("set_permissions")["properties"]["mode"]["type"],
            "string"
        );
        assert_eq!(
            schema("set_permissions")["properties"]["mode"]["pattern"],
            "^[0-7]{3,4}$"
        );
        assert_eq!(
            schema("read_file")["properties"]["length"]["type"],
            "integer"
        );
    }

    /// The level table of the design is a security contract: a later edit must not lower
    /// one silently.
    #[test]
    fn levels_are_pinned() {
        let level = |name: &str| find(name).unwrap().need;
        for name in ["list_sites", "transfer_status"] {
            assert_eq!(level(name), Need::Nothing, "{name}");
        }
        for name in ["list_directory", "stat", "read_file", "download"] {
            assert_eq!(level(name), Need::Read, "{name}");
        }
        for name in [
            "write_file",
            "make_directory",
            "rename",
            "set_permissions",
            "upload",
            "delete",
        ] {
            assert_eq!(level(name), Need::All, "{name}");
        }
        assert_eq!(level("run_command"), Need::AllSsh);
    }

    #[test]
    fn hints_follow_the_table() {
        // (read_only, destructive, idempotent, open_world)
        let table: [(&str, [bool; 4]); 13] = [
            ("list_sites", [true, false, false, false]),
            ("list_directory", [true, false, false, true]),
            ("stat", [true, false, false, true]),
            ("read_file", [true, false, false, true]),
            ("download", [false, true, true, true]),
            ("transfer_status", [true, false, false, false]),
            ("write_file", [false, true, true, true]),
            ("make_directory", [false, false, true, true]),
            ("rename", [false, false, false, true]),
            ("set_permissions", [false, false, true, true]),
            ("upload", [false, true, true, true]),
            ("delete", [false, true, true, true]),
            ("run_command", [false, true, false, true]),
        ];
        for (name, want) in table {
            let t = find(name).unwrap();
            assert_eq!(
                [t.read_only, t.destructive, t.idempotent, t.open_world],
                want,
                "{name}"
            );
        }
        for t in all() {
            if t.need != Need::All && t.need != Need::AllSsh && t.name != "download" {
                assert!(t.read_only, "{} reads, so it is read-only", t.name);
            }
            if t.read_only {
                assert!(!t.destructive, "{}", t.name);
            }
        }
    }

    #[test]
    fn descriptions_name_their_caps() {
        let d = |name: &str| find(name).unwrap().description;
        assert!(d("read_file").contains("1 MiB"));
        assert!(d("list_directory").contains("offset"));
        assert!(d("list_directory").contains("5000"));
        assert!(d("run_command").contains("600"));
        assert!(d("run_command").contains("no terminal"));
        assert!(d("run_command").contains("no input"));
        assert!(d("write_file").contains("1 MiB"));
        for name in ["download", "upload"] {
            assert!(d(name).contains("transfer_status"), "{name}");
            assert!(d(name).contains("\"running\""), "{name}");
            assert!(d(name).contains("overwrite"), "{name}");
        }
        assert!(d("transfer_status").contains("\"running\""));
        for t in all() {
            if t.need != Need::Nothing {
                assert!(t.description.contains("list_sites"), "{}", t.name);
            }
        }
    }

    #[test]
    fn descriptions_state_levels_and_destructiveness() {
        let d = |name: &str| find(name).unwrap().description;
        for name in ["list_directory", "stat", "read_file", "download"] {
            assert!(d(name).contains("Needs read access"), "{name}");
        }
        for name in [
            "write_file",
            "make_directory",
            "rename",
            "set_permissions",
            "upload",
            "delete",
            "run_command",
        ] {
            assert!(d(name).contains("Needs full access"), "{name}");
        }
        assert!(d("run_command").contains("SSH"));
        assert!(d("delete").contains("cannot be undone"));
        assert!(d("write_file").contains("overwrite"));
    }
}
