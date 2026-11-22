//! Command-line arguments. Parsed before anything else starts, so `--version`, `--help`
//! and `mcp` never take the instance lock or open a window.

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    /// No arguments: start the app.
    Run,
    /// `--version` / `-V`.
    Version,
    /// `--help` / `-h`.
    Help,
    /// `mcp`: the relay an MCP client starts (stdio).
    Mcp,
    /// Anything else; carries the first offending argument.
    Unknown(String),
}

/// Parses the arguments after the program name.
pub fn parse_args<I, S>(args: I) -> Cli
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter();
    match args.next() {
        None => Cli::Run,
        Some(first) => match first.as_ref() {
            "--version" | "-V" => Cli::Version,
            "--help" | "-h" => Cli::Help,
            // Exactly `mcp`: anything after it is refused rather than ignored.
            "mcp" => match args.next() {
                None => Cli::Mcp,
                Some(extra) => Cli::Unknown(extra.as_ref().to_owned()),
            },
            other => Cli::Unknown(other.to_owned()),
        },
    }
}

/// The `--version` line.
pub fn version_line() -> String {
    format!("hoprs {}", env!("CARGO_PKG_VERSION"))
}

/// The `--help` text.
pub fn usage() -> &'static str {
    "Usage: hoprs [mcp | --version | --help]"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_version_help_unknown() {
        assert_eq!(parse_args(Vec::<String>::new()), Cli::Run);
        assert_eq!(parse_args(["--version"]), Cli::Version);
        assert_eq!(parse_args(["-V"]), Cli::Version);
        assert_eq!(parse_args(["--help"]), Cli::Help);
        assert_eq!(parse_args(["-h"]), Cli::Help);
        assert_eq!(parse_args(["--bogus"]), Cli::Unknown("--bogus".into()));
        assert_eq!(parse_args(["-V", "x"]), Cli::Version);
    }

    #[test]
    fn parse_args_mcp() {
        assert_eq!(parse_args(["mcp"]), Cli::Mcp);
        assert_eq!(parse_args(["mcp", "x"]), Cli::Unknown("x".into()));
        assert_eq!(parse_args(["MCP"]), Cli::Unknown("MCP".into()));
        assert!(usage().contains("mcp"));
    }
}
