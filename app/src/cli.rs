//! The desktop command line: `immermemo [<dir>] [immermemo://import#...]`.
//!
//! Nothing is required. Which vaults exist and which one is open come from
//! the app's own registry (see `immermemo_core::Session::start`); the
//! arguments only say what to do *in addition* to that.

use std::path::PathBuf;

/// What the command line asked for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Launch {
    /// A folder to open as a vault: registered if it is new, and made the
    /// current vault. `~/` is expanded.
    pub open: Option<PathBuf>,
    /// A deep link (`immermemo://...`), handled as if it had been tapped.
    pub import_uri: Option<String>,
}

/// The usage text, for `--help` and for a mistake.
pub const USAGE: &str = "usage: immermemo [<dir>] [immermemo://import#...]

  <dir>                   open this folder as a vault (registering it if new)
  immermemo://import#...  handle this import link

With no arguments the app opens the vault it was last on.";

/// Reads the arguments after the program name, in any order: an
/// `immermemo://` argument is the deep link, any other argument not starting
/// with `-` is the folder to open. A second of either, or an unknown option,
/// is an error (the message is the problem followed by [`USAGE`]).
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Launch, String> {
    let mut launch = Launch::default();
    for arg in args {
        if arg.starts_with("immermemo://") {
            if launch.import_uri.replace(arg).is_some() {
                return Err(format!("more than one immermemo:// link\n\n{USAGE}"));
            }
        } else if arg.starts_with('-') {
            return Err(format!("unknown option {arg}\n\n{USAGE}"));
        } else if launch
            .open
            .replace(immermemo_core::vault_path_from_input(&arg))
            .is_some()
        {
            return Err(format!("more than one folder\n\n{USAGE}"));
        }
    }
    Ok(launch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Launch, String> {
        parse_args(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn no_arguments_is_fine_and_asks_for_nothing() {
        assert_eq!(parse(&[]), Ok(Launch::default()));
    }

    #[test]
    fn a_folder_and_a_link_are_recognized_in_either_order() {
        let both = Launch {
            open: Some(PathBuf::from("/notes")),
            import_uri: Some("immermemo://import#abc".to_owned()),
        };
        assert_eq!(
            parse(&["/notes", "immermemo://import#abc"]),
            Ok(both.clone())
        );
        assert_eq!(parse(&["immermemo://import#abc", "/notes"]), Ok(both));
        assert_eq!(
            parse(&["immermemo://import#abc"]),
            Ok(Launch {
                open: None,
                import_uri: Some("immermemo://import#abc".to_owned())
            })
        );
    }

    #[test]
    fn a_leading_tilde_in_the_folder_is_expanded() {
        let launch = parse(&["~/notes"]).unwrap();
        let open = launch.open.unwrap();
        assert!(open.is_absolute() || !open.starts_with("~"), "{open:?}");
    }

    #[test]
    fn a_mistake_is_reported_with_the_usage() {
        for args in [
            &["/a", "/b"][..],
            &["immermemo://one", "immermemo://two"],
            &["--vault", "/a"],
        ] {
            let error = parse(args).unwrap_err();
            assert!(error.contains("usage: immermemo"), "{args:?}: {error}");
        }
    }
}
