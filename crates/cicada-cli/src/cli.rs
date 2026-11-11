//! The command line: kubectl-style, verb first.

use cicada::ActorAddress;
use cicada::address::TypeName;
use cicada::store::{MessageId, NodeId, OutcomeStatus};
use cicada::time::{Timestamp, parse_duration, parse_instant};
use clap::{Args, Parser, Subcommand};

use crate::output::Format;

/// Talk to a Cicada node over its HTTP API.
#[derive(Debug, Parser)]
#[command(name = "cicada", version, about, propagate_version = true)]
pub struct Cli {
    /// The context of the config file to use.
    #[arg(long, global = true, help_heading = "Global options")]
    pub context: Option<String>,
    /// Base URL of the API; wins over CICADA_SERVER and the context.
    #[arg(long, global = true, help_heading = "Global options")]
    pub server: Option<String>,
    /// Bearer token; wins over CICADA_TOKEN and the context.
    #[arg(long, global = true, help_heading = "Global options")]
    pub token: Option<String>,
    /// Output format (`describe` defaults to yaml).
    #[arg(short, long, global = true, value_enum, help_heading = "Global options")]
    pub output: Option<Format>,
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// The commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// List or show resources.
    #[command(subcommand)]
    Get(Get),
    /// Show one resource in full.
    #[command(subcommand)]
    Describe(Describe),
    /// Create an actor.
    #[command(subcommand)]
    Create(Create),
    /// Delete an actor or a type registration.
    #[command(subcommand)]
    Delete(Delete),
    /// Send a message to an actor.
    Send(SendArgs),
    /// Cancel a message, messages, or a running turn.
    #[command(subcommand)]
    Cancel(Cancel),
    /// Send failed or cancelled messages again.
    #[command(subcommand)]
    Retry(Retry),
    /// Make a pending message due now.
    DeliverNow {
        /// The message id.
        #[arg(value_parser = message_id)]
        id: MessageId,
    },
    /// Pause an actor's mailbox: messages queue up, no turn starts.
    Pause {
        /// The actor, `type/key`.
        #[arg(value_parser = address)]
        actor: ActorAddress,
    },
    /// Resume a paused actor.
    Resume {
        /// The actor, `type/key`.
        #[arg(value_parser = address)]
        actor: ActorAddress,
    },
    /// Give outcome `purged` to every pending message of an actor.
    Purge {
        /// The actor, `type/key`.
        #[arg(value_parser = address)]
        actor: ActorAddress,
    },
    /// Count actors and pending messages under a prefix.
    Stats {
        /// Address prefix.
        #[arg(long, default_value = "")]
        prefix: String,
    },
    /// Whether the node runs and its store answers.
    Health,
    /// Read and change the config file.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Browse and act in a terminal UI.
    Tui,
}

/// Paging of a listing.
#[derive(Debug, Clone, Copy, Args)]
pub struct Page {
    /// Items per page (1..1000).
    #[arg(long, default_value_t = 100)]
    pub limit: usize,
    /// Follow the cursor to the last page.
    #[arg(long)]
    pub all: bool,
}

/// `get` resources.
#[derive(Debug, Subcommand)]
pub enum Get {
    /// Actor addresses under a prefix.
    Actors {
        /// Address prefix.
        #[arg(long, default_value = "")]
        prefix: String,
        /// Paging.
        #[command(flatten)]
        page: Page,
    },
    /// One actor.
    Actor {
        /// The actor, `type/key`.
        #[arg(value_parser = address)]
        actor: ActorAddress,
    },
    /// The direct children of one actor, in address order: the actors its turns created as children.
    Children {
        /// The actor, `type/key`.
        #[arg(value_parser = address)]
        actor: ActorAddress,
        /// Paging.
        #[command(flatten)]
        page: Page,
    },
    /// The pending messages of one actor, in delivery order.
    Mailbox {
        /// The actor, `type/key`.
        #[arg(value_parser = address)]
        actor: ActorAddress,
        /// Paging.
        #[command(flatten)]
        page: Page,
    },
    /// The pending messages of every actor under a prefix.
    Messages {
        /// Address prefix.
        #[arg(long, default_value = "")]
        prefix: String,
        /// Paging.
        #[command(flatten)]
        page: Page,
    },
    /// One message: pending, running or finished.
    Message {
        /// The message id.
        #[arg(value_parser = message_id)]
        id: MessageId,
    },
    /// The messages sent by the turn that handled a message.
    Caused {
        /// The message id.
        #[arg(value_parser = message_id)]
        id: MessageId,
        /// Paging.
        #[command(flatten)]
        page: Page,
    },
    /// Finished messages.
    Outcomes {
        /// Only this status: done, failed, cancelled or purged.
        #[arg(long, value_parser = outcome_status)]
        status: Option<OutcomeStatus>,
        /// Address prefix.
        #[arg(long, default_value = "")]
        prefix: String,
        /// Only outcomes from this instant on (ISO-8601).
        #[arg(long, value_parser = instant)]
        since: Option<Timestamp>,
        /// Paging.
        #[command(flatten)]
        page: Page,
    },
    /// The turns running now.
    Turns {
        /// Only turns of this node.
        #[arg(long, value_parser = node_id)]
        node: Option<NodeId>,
        /// Address prefix.
        #[arg(long, default_value = "")]
        prefix: String,
        /// Paging.
        #[command(flatten)]
        page: Page,
    },
    /// The turn running now on one actor.
    Turn {
        /// The actor, `type/key`.
        #[arg(value_parser = address)]
        actor: ActorAddress,
    },
    /// Every node.
    Nodes,
    /// One node.
    Node {
        /// The node id.
        #[arg(value_parser = node_id)]
        id: NodeId,
    },
    /// Every registered type.
    Types,
    /// One registered type.
    Type {
        /// The type name.
        #[arg(value_parser = type_name)]
        name: TypeName,
    },
}

/// `describe` resources.
#[derive(Debug, Subcommand)]
pub enum Describe {
    /// One actor with its state.
    Actor {
        /// The actor, `type/key`.
        #[arg(value_parser = address)]
        actor: ActorAddress,
    },
    /// One message with its envelope.
    Message {
        /// The message id.
        #[arg(value_parser = message_id)]
        id: MessageId,
    },
    /// One node with its running turns.
    Node {
        /// The node id.
        #[arg(value_parser = node_id)]
        id: NodeId,
    },
    /// One type with its JSON Schemas.
    Type {
        /// The type name.
        #[arg(value_parser = type_name)]
        name: TypeName,
    },
}

/// `create` resources.
#[derive(Debug, Subcommand)]
pub enum Create {
    /// Create an actor; an existing one is left untouched.
    Actor {
        /// The actor, `type/key`.
        #[arg(value_parser = address)]
        actor: ActorAddress,
        /// Initial state as JSON; `'{}'` for a type that keeps no data.
        #[arg(long, value_parser = json)]
        state: serde_json::Value,
    },
}

/// `delete` resources.
#[derive(Debug, Subcommand)]
pub enum Delete {
    /// Delete an actor and every descendant (its children, theirs, and so on), and purge their mailboxes.
    Actor {
        /// The actor, `type/key`.
        #[arg(value_parser = address)]
        actor: ActorAddress,
    },
    /// Remove a type's registration; refused while actors of the type exist.
    Type {
        /// The type name.
        #[arg(value_parser = type_name)]
        name: TypeName,
    },
}

/// `send` arguments.
#[derive(Debug, Args)]
pub struct SendArgs {
    /// The target actor, `type/key`.
    #[arg(value_parser = address)]
    pub actor: ActorAddress,
    /// The message type name.
    #[arg(long = "type")]
    pub message_type: String,
    /// The payload as JSON.
    #[arg(long, value_parser = json, default_value = "{}")]
    pub payload: serde_json::Value,
    /// The message id (dedup key); a fresh one when absent.
    #[arg(long, value_parser = message_id)]
    pub id: Option<MessageId>,
    /// Due after this ISO-8601 duration, e.g. PT5S.
    #[arg(long, value_parser = duration, conflicts_with = "at")]
    pub delay: Option<std::time::Duration>,
    /// Due at this ISO-8601 instant.
    #[arg(long, value_parser = instant)]
    pub at: Option<Timestamp>,
    /// Correlation id, passed through.
    #[arg(long)]
    pub correlation_id: Option<String>,
    /// A header `name=value`, passed through; repeatable.
    #[arg(long = "header", value_parser = pair)]
    pub headers: Vec<(String, String)>,
}

/// Which messages a bulk action takes: ids, or a filter.
#[derive(Debug, Args)]
#[group(required = true, multiple = true)]
pub struct Selection {
    /// A message id; repeatable, at most 1000.
    #[arg(long = "id", value_parser = message_id, conflicts_with_all = ["prefix", "message_type"])]
    pub ids: Vec<MessageId>,
    /// Messages of actors under this address prefix.
    #[arg(long)]
    pub prefix: Option<String>,
    /// Messages of this type, under `--prefix`.
    #[arg(long = "type", requires = "prefix")]
    pub message_type: Option<String>,
}

/// `cancel` targets.
#[derive(Debug, Subcommand)]
pub enum Cancel {
    /// Cancel a pending message.
    Message {
        /// The message id.
        #[arg(value_parser = message_id)]
        id: MessageId,
    },
    /// Cancel messages by ids or by filter, at most 1000 per call.
    Messages(Selection),
    /// Ask the owning node to stop the turn running on an actor.
    Turn {
        /// The actor, `type/key`.
        #[arg(value_parser = address)]
        actor: ActorAddress,
    },
}

/// `retry` targets.
#[derive(Debug, Subcommand)]
pub enum Retry {
    /// Send a failed or cancelled message again, as a new message.
    Message {
        /// The message id.
        #[arg(value_parser = message_id)]
        id: MessageId,
    },
    /// Retry messages by ids or by filter (over failed outcomes), at most 1000 per call.
    Messages(Selection),
}

/// `config` commands. `use-context` and `set-context` rewrite the whole file, so comments in it are not kept.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// List the contexts; `*` marks the current one.
    GetContexts,
    /// Print the current context.
    CurrentContext,
    /// Make a context the current one. Rewrites the whole file: comments in it are not kept.
    UseContext {
        /// The context name.
        name: String,
    },
    /// Add or replace a context, with the global `--server` (required) and `--token`. Rewrites the whole file:
    /// comments in it are not kept.
    SetContext {
        /// The context name.
        name: String,
        /// A header `name=value`; repeatable.
        #[arg(long = "header", value_parser = pair)]
        headers: Vec<(String, String)>,
    },
}

fn address(text: &str) -> Result<ActorAddress, String> {
    ActorAddress::parse(text).map_err(|e| e.to_string())
}

fn message_id(text: &str) -> Result<MessageId, String> {
    MessageId::new(text).map_err(|e| e.to_string())
}

fn node_id(text: &str) -> Result<NodeId, String> {
    NodeId::new(text).map_err(|e| e.to_string())
}

fn type_name(text: &str) -> Result<TypeName, String> {
    TypeName::parse(text).map_err(|e| e.to_string())
}

fn outcome_status(text: &str) -> Result<OutcomeStatus, String> {
    OutcomeStatus::from_wire_name(text).ok_or_else(|| "expected done, failed, cancelled or purged".to_owned())
}

fn instant(text: &str) -> Result<Timestamp, String> {
    parse_instant(text).map_err(|e| e.to_string())
}

fn duration(text: &str) -> Result<std::time::Duration, String> {
    parse_duration(text).map_err(|e| e.to_string())
}

fn json(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))
}

fn pair(text: &str) -> Result<(String, String), String> {
    text.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())).ok_or_else(|| "expected name=value".to_owned())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_command_tree_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn a_bulk_action_needs_ids_or_a_filter_not_both() {
        assert!(Cli::try_parse_from(["cicada", "cancel", "messages"]).is_err());
        assert!(Cli::try_parse_from(["cicada", "cancel", "messages", "--id", "a", "--prefix", "x/"]).is_err());
        assert!(Cli::try_parse_from(["cicada", "cancel", "messages", "--prefix", "x/", "--type", "T"]).is_ok());
    }

    #[test]
    fn a_bulk_filter_by_type_needs_a_prefix() {
        for verb in ["cancel", "retry"] {
            assert!(Cli::try_parse_from(["cicada", verb, "messages", "--type", "T"]).is_err(), "{verb}");
            assert!(Cli::try_parse_from(["cicada", verb, "messages", "--prefix", "x/"]).is_ok(), "{verb}");
        }
    }

    #[test]
    fn the_commands_that_rewrite_the_config_file_say_that_comments_are_lost() {
        let mut cli = Cli::command();
        let config = cli.find_subcommand_mut("config").unwrap();
        for name in ["use-context", "set-context"] {
            let help = config.find_subcommand_mut(name).unwrap().render_long_help().to_string();
            assert!(help.contains("comments in it are not kept"), "{name}: {help}");
        }
    }

    #[test]
    fn the_help_of_delete_actor_says_that_the_descendants_go_too() {
        let mut cli = Cli::command();
        let delete = cli.find_subcommand_mut("delete").unwrap();
        let help = delete.find_subcommand_mut("actor").unwrap().render_long_help().to_string();
        assert!(help.contains("every descendant"), "{help}");
    }

    #[test]
    fn delay_and_at_exclude_each_other() {
        let args = ["cicada", "send", "a/b", "--type", "T", "--delay", "PT5S", "--at", "2026-01-01T00:00:00Z"];
        assert!(Cli::try_parse_from(args).is_err());
    }
}
