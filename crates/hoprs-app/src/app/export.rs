//! The export screen's helpers: the default file name and the title of what is exported.

// Nothing calls these until the export screen (Task 3); remove this then.
#![allow(dead_code)]

use chrono::NaiveDate;
use hoprs_vault::ExportScope;

use super::App;

/// The longest slug in a default file name, in characters.
const SLUG_MAX: usize = 40;

/// A name made safe for a file name: lowercase, every run of characters that are not
/// Unicode alphanumerics becomes one `-`, no `-` at either end, at most 40 characters;
/// `export` when nothing is left.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
        } else {
            pending_dash = true;
        }
    }
    let mut out: String = out.chars().take(SLUG_MAX).collect();
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() { "export".into() } else { out }
}

/// `hoprs-<slug>-<YYYY-MM-DD>.hoprs`, or `hoprs-backup-<date>.hoprs` without a name.
pub fn default_file_name(name: Option<&str>, today: NaiveDate) -> String {
    let what = name.map_or_else(|| "backup".to_owned(), slug);
    format!("hoprs-{what}-{}.hoprs", today.format("%Y-%m-%d"))
}

/// What the export screen says it exports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportTitle {
    Site { name: String },
    Workspace { name: String, sites: usize },
    All { sites: usize, workspaces: usize },
}

impl ExportTitle {
    /// The site or workspace name; `None` for everything.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Site { name } | Self::Workspace { name, .. } => Some(name),
            Self::All { .. } => None,
        }
    }
}

impl App {
    /// The title for `scope`; `None` when the site or workspace is not in the profiles.
    pub fn export_title(&self, scope: &ExportScope) -> Option<ExportTitle> {
        match scope {
            ExportScope::Site(id) => self.site(id).map(|site| ExportTitle::Site {
                name: site.name.clone(),
            }),
            ExportScope::Workspace(id) => {
                self.workspace(id).map(|workspace| ExportTitle::Workspace {
                    name: workspace.name.clone(),
                    sites: workspace
                        .sites
                        .iter()
                        .filter(|entry| self.site(&entry.site).is_some())
                        .count(),
                })
            }
            ExportScope::All => Some(ExportTitle::All {
                sites: self.sites.len(),
                workspaces: self.workspaces.len(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn slug_rules() {
        assert_eq!(slug("Cliente Acme"), "cliente-acme");
        assert_eq!(slug("São Paulo / prod"), "são-paulo-prod");
        assert_eq!(slug("a/b\\c:d"), "a-b-c-d");
        for empty in ["  --  ", "///", "🙂", ""] {
            assert_eq!(slug(empty), "export", "{empty:?}");
        }
        assert_eq!(slug(&"a".repeat(200)), "a".repeat(40));
        let cut = slug(&format!("{} x", "a".repeat(39)));
        assert_eq!(cut, "a".repeat(39));
        assert!(!cut.ends_with('-'));
    }

    #[test]
    fn default_file_names() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        assert_eq!(
            default_file_name(Some("Cliente Acme"), day),
            "hoprs-cliente-acme-2026-11-22.hoprs"
        );
        assert_eq!(
            default_file_name(None, day),
            "hoprs-backup-2026-11-22.hoprs"
        );
    }
}
