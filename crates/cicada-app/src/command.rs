//! The command line of a binary built from [`crate::App`]: the operator commands of [`crate::cli`], and `serve` and
//! `worker` in a binary that hosts types, named after the binary as it was called.

use std::ffi::OsString;
use std::path::Path;

use clap::error::ErrorKind;
use clap::parser::ValueSource;
use clap::{Arg, ArgMatches, Command, CommandFactory, FromArgMatches};

use crate::cli::Cli;
use crate::settings::{ServeArgs, WorkerArgs};

/// What a command line asks for.
pub(crate) enum Parsed {
    /// An operator command.
    Operator(Box<Cli>),
    /// `serve`.
    Serve(Box<ServeArgs>),
    /// `worker`.
    Worker(Box<WorkerArgs>),
    /// The help, rendered.
    Help(String),
    /// The version.
    Version,
}

/// The flags of the operator commands, by id, which `serve` and `worker` refuse.
const OPERATOR_FLAGS: [(&str, &str); 4] =
    [("context", "--context"), ("server", "--server"), ("token", "--token"), ("output", "-o")];

/// The name of the binary as it was called: the file name of `args[0]`.
pub(crate) fn program_name(args: &[OsString]) -> String {
    args.first()
        .and_then(|first| Path::new(first).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cicada".to_owned())
}

/// The command tree of a binary called `name`; with `node`, it has `serve` and `worker`.
pub(crate) fn command(name: &str, node: bool) -> Command {
    let command = Cli::command().name(name.to_owned()).bin_name(name.to_owned());
    if !node {
        return command;
    }
    let serve = ServeArgs::command()
        .about("Host the types and serve the HTTP API, until SIGTERM or SIGINT; it has no authentication of its own")
        .display_order(0);
    let worker =
        WorkerArgs::command().about("Host the types and serve nothing, until SIGTERM or SIGINT").display_order(0);
    command
        .about("Run a Cicada node, or talk to one over its HTTP API.")
        .subcommand(hide_operator_flags(serve))
        .subcommand(hide_operator_flags(worker))
}

/// The flags of the operator commands, hidden: clap then does not give the global ones to `command`, so its help does
/// not show them, and [`refuse_operator_flags`] still sees them.
fn hide_operator_flags(command: Command) -> Command {
    command.args(OPERATOR_FLAGS.map(|(id, flag)| {
        let arg = Arg::new(id).value_name("VALUE").hide(true);
        match flag.strip_prefix("--") {
            Some(long) => arg.long(long),
            None => arg.short('o').long("output"),
        }
    }))
}

/// Reads `args`, which start with the name of the program; `node` says whether the binary hosts types. An error is the
/// message of a command line that cannot be used, without clap's `error: `.
pub(crate) fn parse(args: &[OsString], node: bool) -> Result<Parsed, String> {
    let name = program_name(args);
    let mut command = command(&name, node);
    let matches = match command.try_get_matches_from_mut(args) {
        Ok(matches) => matches,
        Err(e) => {
            return match e.kind() {
                ErrorKind::DisplayHelp => Ok(Parsed::Help(e.render().to_string())),
                ErrorKind::DisplayVersion => Ok(Parsed::Version),
                ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                    Err(hide_passwords(&format!("a command is missing\n\n{}", e.render().to_string().trim_end())))
                }
                _ => Err(usage(&e)),
            };
        }
    };
    match matches.subcommand() {
        Some(("serve", sub)) => {
            refuse_operator_flags("serve", &matches, sub)?;
            ServeArgs::from_arg_matches(sub).map(|args| Parsed::Serve(Box::new(args))).map_err(|e| usage(&e))
        }
        Some(("worker", sub)) => {
            refuse_operator_flags("worker", &matches, sub)?;
            WorkerArgs::from_arg_matches(sub).map(|args| Parsed::Worker(Box::new(args))).map_err(|e| usage(&e))
        }
        _ => Cli::from_arg_matches(&matches).map(|cli| Parsed::Operator(Box::new(cli))).map_err(|e| usage(&e)),
    }
}

/// The flags of the operator commands are global, so clap takes them before `serve` and `worker` too, and after them as
/// the hidden flags of [`hide_operator_flags`]; there they would do nothing, and `serve --token` would look like
/// protection.
fn refuse_operator_flags(command: &str, top: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let given: Vec<&str> = OPERATOR_FLAGS
        .iter()
        .filter(|(id, _)| [top, sub].iter().any(|m| m.value_source(id) == Some(ValueSource::CommandLine)))
        .map(|(_, flag)| *flag)
        .collect();
    if given.is_empty() {
        return Ok(());
    }
    let why = match command {
        "serve" => "serve has no authentication of its own: a host protects the API with a layer in App::http",
        _ => "worker serves no HTTP",
    };
    Err(format!(
        "{command} does not take {}: --context, --server, --token and -o are flags of the operator commands, and {why}",
        given.join(", ")
    ))
}

/// The text of a clap error without its leading `error: ` and its trailing newline, and without passwords: clap quotes
/// what it cannot use, such as an address given without `--store`.
fn usage(error: &clap::Error) -> String {
    let text = error.render().to_string();
    let text = text.strip_prefix("error: ").unwrap_or(&text);
    hide_passwords(text.trim_end())
}

/// `text` with the password of every address in it that has one (`scheme://user:secret@host` becomes
/// `scheme://user:***@host`).
pub(crate) fn hide_passwords(text: &str) -> String {
    let mut hidden = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("://") {
        let (before, after) = rest.split_at(i + 3);
        hidden.push_str(before);
        let end = after
            .find(|c: char| c.is_whitespace() || matches!(c, '/' | '?' | '#' | '\'' | '"' | '`'))
            .unwrap_or(after.len());
        let authority = &after[..end];
        match authority.rfind('@').and_then(|at| authority[..at].find(':').map(|colon| (colon, at))) {
            Some((colon, at)) => {
                hidden.push_str(&authority[..=colon]);
                hidden.push_str("***");
                hidden.push_str(&authority[at..]);
            }
            None => hidden.push_str(authority),
        }
        rest = &after[end..];
    }
    hidden.push_str(rest);
    hidden
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_password_of_an_address_is_hidden() {
        assert_eq!(
            hide_passwords("unexpected argument 'postgres://u:pw@h:5432/db?sslmode=require' found"),
            "unexpected argument 'postgres://u:***@h:5432/db?sslmode=require' found"
        );
        assert_eq!(hide_passwords("a://x:p@q@h b://n@h c://h:1 d://u:@h"), "a://x:***@h b://n@h c://h:1 d://u:***@h");
        assert_eq!(hide_passwords("no address here"), "no address here");
    }

    #[test]
    fn the_command_trees_are_consistent() {
        command("myapp", false).debug_assert();
        command("myapp", true).debug_assert();
    }

    #[test]
    fn the_name_is_the_file_name_of_the_first_argument() {
        assert_eq!(program_name(&args(&["/usr/local/bin/myapp", "get", "actors"])), "myapp");
        assert_eq!(program_name(&args(&["myapp"])), "myapp");
        assert_eq!(program_name(&[]), "cicada");
    }

    #[test]
    fn the_help_names_the_binary_as_it_was_called() {
        let Ok(Parsed::Help(help)) = parse(&args(&["/opt/myapp", "--help"]), true) else { panic!("help") };
        assert!(help.contains("Usage: myapp") && help.contains("\n  serve ") && help.contains("\n  worker "), "{help}");
        let Ok(Parsed::Help(help)) = parse(&args(&["cicada", "--help"]), false) else { panic!("help") };
        assert!(!help.contains("\n  serve ") && !help.contains("\n  worker "), "{help}");
    }

    #[test]
    fn an_unknown_command_is_a_usage_error_without_the_prefix() {
        let Err(error) = parse(&args(&["myapp", "nope"]), true) else { panic!("an error") };
        assert!(error.starts_with("unrecognized subcommand 'nope'"), "{error}");
        assert!(error.contains("Usage: myapp"), "{error}");
    }

    #[test]
    fn no_command_is_a_usage_error_with_the_help() {
        let Err(error) = parse(&args(&["myapp"]), true) else { panic!("an error") };
        assert!(error.starts_with("a command is missing") && error.contains("Usage: myapp"), "{error}");
    }

    /// Scenario: An operator flag on `serve`.
    #[test]
    fn serve_and_worker_refuse_the_flags_of_the_operator_commands() {
        for line in [
            &["myapp", "serve", "--token", "secret"][..],
            &["myapp", "--token", "secret", "serve"],
            &["myapp", "serve", "--server", "http://x"],
            &["myapp", "serve", "--context", "prod"],
            &["myapp", "serve", "-o", "json"],
        ] {
            let Err(error) = parse(&args(line), true) else { panic!("{line:?}: an error") };
            assert!(error.starts_with("serve does not take "), "{line:?}: {error}");
            assert!(error.contains("no authentication of its own") && error.contains("App::http"), "{error}");
        }
        let Err(error) = parse(&args(&["myapp", "worker", "--token", "secret"]), true) else { panic!("an error") };
        assert!(error.starts_with("worker does not take --token"), "{error}");
    }
}
