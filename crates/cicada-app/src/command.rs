//! The command line of a binary built from [`crate::App`]: the operator commands of [`crate::cli`], named after the
//! binary as it was called.

use std::ffi::OsString;
use std::path::Path;

use clap::error::ErrorKind;
use clap::{Command, CommandFactory, FromArgMatches};

use crate::cli::Cli;

/// What a command line asks for.
pub(crate) enum Parsed {
    /// An operator command.
    Operator(Box<Cli>),
    /// The help, rendered.
    Help(String),
    /// The version.
    Version,
}

/// The name of the binary as it was called: the file name of `args[0]`.
pub(crate) fn program_name(args: &[OsString]) -> String {
    args.first()
        .and_then(|first| Path::new(first).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cicada".to_owned())
}

/// The command tree of a binary called `name`.
pub(crate) fn command(name: &str) -> Command {
    Cli::command().name(name.to_owned()).bin_name(name.to_owned())
}

/// Reads `args`, which start with the name of the program. An error is the message of a command line that cannot be
/// used, without clap's `error: `.
pub(crate) fn parse(args: &[OsString]) -> Result<Parsed, String> {
    let name = program_name(args);
    let mut command = command(&name);
    let matches = match command.try_get_matches_from_mut(args) {
        Ok(matches) => matches,
        Err(e) => {
            return match e.kind() {
                ErrorKind::DisplayHelp => Ok(Parsed::Help(e.render().to_string())),
                ErrorKind::DisplayVersion => Ok(Parsed::Version),
                ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                    Err(format!("a command is missing\n\n{}", e.render().to_string().trim_end()))
                }
                _ => Err(usage(&e)),
            };
        }
    };
    Cli::from_arg_matches(&matches).map(|cli| Parsed::Operator(Box::new(cli))).map_err(|e| usage(&e))
}

/// The text of a clap error without its leading `error: ` and its trailing newline.
fn usage(error: &clap::Error) -> String {
    let text = error.render().to_string();
    let text = text.strip_prefix("error: ").unwrap_or(&text);
    text.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_command_tree_is_consistent() {
        command("myapp").debug_assert();
    }

    #[test]
    fn the_name_is_the_file_name_of_the_first_argument() {
        assert_eq!(program_name(&args(&["/usr/local/bin/myapp", "get", "actors"])), "myapp");
        assert_eq!(program_name(&args(&["myapp"])), "myapp");
        assert_eq!(program_name(&[]), "cicada");
    }

    #[test]
    fn the_help_names_the_binary_as_it_was_called() {
        let Ok(Parsed::Help(help)) = parse(&args(&["/opt/myapp", "--help"])) else { panic!("help") };
        assert!(help.contains("Usage: myapp"), "{help}");
    }

    #[test]
    fn an_unknown_command_is_a_usage_error_without_the_prefix() {
        let Err(error) = parse(&args(&["myapp", "nope"])) else { panic!("an error") };
        assert!(error.starts_with("unrecognized subcommand 'nope'"), "{error}");
        assert!(error.contains("Usage: myapp"), "{error}");
    }

    #[test]
    fn no_command_is_a_usage_error_with_the_help() {
        let Err(error) = parse(&args(&["myapp"])) else { panic!("an error") };
        assert!(error.starts_with("a command is missing") && error.contains("Usage: myapp"), "{error}");
    }
}
