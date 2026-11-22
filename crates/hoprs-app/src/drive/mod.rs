//! Google Drive as a second destination for the encrypted export file.

mod client;
mod config;
mod loopback;
mod oauth;

/// A file in the user's Google Drive, as the upload reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
// TODO(task 10): the export flow reads these fields; remove this allow then.
#[allow(dead_code)]
pub struct DriveFile {
    pub name: String,
    /// The file's page in Google Drive, only when it is an `https://` link on `google.com`.
    pub link: Option<String>,
}
