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

/// The flags of the operator commands, by id, which `serve` refuses.
const OPERATOR_FLAGS: [(&str, &str); 4] =
    [("context", "--context"), ("server", "--server"), ("token", "--token"), ("output", "-o")];
/// The flag of the operator commands that `worker` refuses: it takes the others, for HTTP mode (ADR 0042).
const OUTPUT_FLAG: [(&str, &str); 1] = [("output", "-o")];

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
    let worker = WorkerArgs::command()
        .about("Host the types and serve nothing, on a store or through a server's HTTP API, until SIGTERM or SIGINT")
        .after_help(WORKER_HELP)
        .display_order(0);
    command
        .about("Run a Cicada node, or talk to one over its HTTP API.")
        .subcommand(hide_flags(serve, &OPERATOR_FLAGS))
        .subcommand(hide_flags(worker, &OUTPUT_FLAG))
}

/// What the help of `worker` says after its flags.
const WORKER_HELP: &str = "\
Mode: --store runs the turns on the store, --server or --context through a server's HTTP API; not both.
Without either, a non-blank CICADA_STORE selects the store, else a non-blank CICADA_SERVER or CICADA_CONTEXT
selects HTTP; a token, or the current context of the config file, alone selects nothing.

Over HTTP the server owns the store and the leases: --store-schema, --store-pool-size, --retention, --lease-ttl,
--heartbeat-interval and --max-lost-claims are refused, and their variables are not read. The server, the token
and the headers come as for the operator commands. A binary with App::on_start or App::task cannot run over HTTP.";

/// The flags `flags` of the operator commands, hidden: clap then does not give the global ones to `command`, so its
/// help does not show them, and [`refuse_flags`] still sees them.
fn hide_flags(command: Command, flags: &[(&'static str, &'static str)]) -> Command {
    command.args(flags.iter().map(|&(id, flag)| {
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
            refuse_flags("serve", &OPERATOR_FLAGS, &matches, sub)?;
            ServeArgs::from_arg_matches(sub).map(|args| Parsed::Serve(Box::new(args))).map_err(|e| usage(&e))
        }
        Some(("worker", sub)) => {
            refuse_flags("worker", &OUTPUT_FLAG, &matches, sub)?;
            let mut args = WorkerArgs::from_arg_matches(sub).map_err(|e| usage(&e))?;
            // A global flag before `worker` is in the matches of the top: the one after `worker` wins.
            let top = |id: &str| matches.get_one::<String>(id).cloned();
            args.server = args.server.or_else(|| top("server"));
            args.context = args.context.or_else(|| top("context"));
            args.token = args.token.or_else(|| top("token"));
            Ok(Parsed::Worker(Box::new(args)))
        }
        _ => Cli::from_arg_matches(&matches).map(|cli| Parsed::Operator(Box::new(cli))).map_err(|e| usage(&e)),
    }
}

/// The flags of the operator commands are global, so clap takes them before `serve` and `worker` too, and after them as
/// the hidden flags of [`hide_flags`]; there `flags` would do nothing, and `serve --token` would look like protection.
fn refuse_flags(command: &str, flags: &[(&str, &str)], top: &ArgMatches, sub: &ArgMatches) -> Result<(), String> {
    let given: Vec<&str> = flags
        .iter()
        .filter(|(id, _)| [top, sub].iter().any(|m| m.value_source(id) == Some(ValueSource::CommandLine)))
        .map(|(_, flag)| *flag)
        .collect();
    if given.is_empty() {
        return Ok(());
    }
    if command == "serve" {
        return Err(format!(
            "serve does not take {}: --context, --server, --token and -o are flags of the operator commands, and serve \
             has no authentication of its own: a host protects the API with a layer in App::http",
            given.join(", ")
        ));
    }
    Err(format!("{command} does not take {}: it prints no results, only its log", given.join(", ")))
}

/// The text of a clap error without its leading `error: ` and its trailing newline, and without passwords: clap quotes
/// what it cannot use, such as an address given without `--store`.
fn usage(error: &clap::Error) -> String {
    let text = error.render().to_string();
    let text = text.strip_prefix("error: ").unwrap_or(&text);
    hide_passwords(text.trim_end())
}

/// `text` with the password of every address in it that has one (`scheme://user:secret@host` becomes
/// `scheme://user:***@host`): everything between the first `:` and the last `@` of the authority, also a raw `'` or
/// another sub-delimiter, so that a quote of clap or a following address never cuts the password short. Where the end
/// of an address is unclear it hides more, not less, and hiding twice gives what hiding once gave. It leaves alone a
/// password with a raw `/`, `#`, `?`, `"`, `` ` `` or a space, which end the authority, an authority without `@`, an
/// address without `//` (`postgres:u:pw@h`) and the libpq form (`host=h password=…`).
pub(crate) fn hide_passwords(text: &str) -> String {
    let mut hidden = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("://") {
        let (before, after) = rest.split_at(i + 3);
        hidden.push_str(before);
        let boundary =
            after.find(|c: char| c.is_whitespace() || matches!(c, '/' | '?' | '#' | '"' | '`')).unwrap_or(after.len());
        // A following address (`a://h,b://u:pw@k`) starts a scan of its own: its scheme is not this one's port.
        let end = match after[..boundary].strip_suffix(':') {
            Some(prefix) if after[boundary..].starts_with("//") => {
                let scheme = prefix.bytes().rev().take_while(|b| b.is_ascii_alphanumeric() || b"+-.".contains(b));
                prefix.len() - scheme.count()
            }
            _ => boundary,
        };
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

    /// Each pair is a text and what is printed of it; hiding the result again changes nothing.
    #[test]
    fn a_password_is_hidden_also_with_an_apostrophe_or_a_following_address() {
        let cases = [
            // A raw `'` in the password, the user name or just before the `@`, also inside clap's quotes.
            (
                "unexpected argument 'postgres://u:S3'CRET@h/db' found",
                "unexpected argument 'postgres://u:***@h/db' found",
            ),
            ("postgres://o'neil:S3'CRET@h/db", "postgres://o'neil:***@h/db"),
            ("'a://u:pw'@h'", "'a://u:***@h'"),
            // Sub-delimiters, percent escapes and Unicode, copied as they are.
            ("a://u:!$&'()*+,;=:@h:1/x", "a://u:***@h:1/x"),
            ("a://u%40x:p%27q@h", "a://u%40x:***@h"),
            ("a://ü:pässwörd@hôst/ß", "a://ü:***@hôst/ß"),
            // Empty and multiple `@`: the last `@` ends the password.
            ("a://u:@h", "a://u:***@h"),
            ("a://u:p@q@h", "a://u:***@h"),
            // Adjacent addresses, quoted or after an empty name of --types.
            ("'a://u:p'q@h','b://v:r's@k'", "'a://u:***@h','b://v:***@k'"),
            ("a://h,,b://u:secret@k", "a://h,,b://u:***@k"),
            ("a://h/x,b+c.d-e://u:p@k,c://h:1", "a://h/x,b+c.d-e://u:***@k,c://h:1"),
            ("a://u:p@h,://x", "a://u:***@h,://x"),
            // No password, a port, IPv6, an authority without `@`, plain text.
            (
                "a://user@host b://host:5432 c://[::1]:5432/db d://u@[::1]:1",
                "a://user@host b://host:5432 c://[::1]:5432/db d://u@[::1]:1",
            ),
            ("a://u:p@[::1]:5432/db", "a://u:***@[::1]:5432/db"),
            ("a://u:secret", "a://u:secret"),
            ("ünïcode, no address: here", "ünïcode, no address: here"),
        ];
        for (text, printed) in cases {
            let once = hide_passwords(text);
            assert_eq!(once, printed, "{text}");
            assert_eq!(hide_passwords(&once), once, "{text}: twice");
        }
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
        for line in [&["myapp", "worker", "-o", "json"][..], &["myapp", "-o", "json", "worker"]] {
            let Err(error) = parse(&args(line), true) else { panic!("{line:?}: an error") };
            assert!(error.starts_with("worker does not take -o"), "{line:?}: {error}");
        }
    }

    /// The worker flags of HTTP mode, before or after `worker`.
    #[test]
    fn worker_takes_the_server_context_and_token_before_or_after_the_command() {
        for line in [
            &["myapp", "worker", "--server", "http://x", "--context", "c", "--token", "t"][..],
            &["myapp", "--server", "http://x", "--context", "c", "--token", "t", "worker"],
            &["myapp", "--server", "http://x", "worker", "--context", "c", "--token", "t"],
        ] {
            let Ok(Parsed::Worker(worker)) = parse(&args(line), true) else { panic!("{line:?}: a worker") };
            let given = (worker.server.as_deref(), worker.context.as_deref(), worker.token.as_deref());
            assert_eq!(given, (Some("http://x"), Some("c"), Some("t")), "{line:?}");
        }
        let Ok(Parsed::Worker(worker)) =
            parse(&args(&["myapp", "--server", "http://a", "worker", "--server", "http://b"]), true)
        else {
            panic!("a worker")
        };
        assert_eq!(worker.server.as_deref(), Some("http://b"), "the flag after the command wins");
        assert!(crate::node_command(&args(&["myapp", "--server", "http://x", "worker"])));
        assert!(crate::node_command(&args(&["myapp", "worker", "--context", "prod"])));
        let Ok(Parsed::Help(help)) = parse(&args(&["myapp", "worker", "--help"]), true) else { panic!("help") };
        assert!(help.contains("--server <URL>") && help.contains("--token <TOKEN>"), "{help}");
        assert!(!help.contains("--output"), "{help}");
    }
}
