//! The TUI's state and input handling, without I/O: keys, mouse, ticks and results go in, [`Effect`]s come out.
//!
//! Screens form a stack: Enter drills down (pushes), Esc goes back (pops), a `:` command replaces the stack.

use cicada::ActorAddress;
use cicada::address::TypeName;
use cicada::api::{NodeView, StatsView};
use cicada::store::{MessageId, NodeId, OutcomeStatus};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use serde_json::Value;

use super::grid::{Grid, Rows};
use super::theme::Tone;

/// Ticks (of 250 ms) between two fetches of the current screen.
pub const TICKS_PER_REFRESH: u64 = 8;
/// Ticks between two header summaries; `GET /stats` reads every row, so not too often.
pub const TICKS_PER_SUMMARY: u64 = 40;
const TOAST_TICKS: u64 = 20;
const ERROR_TICKS: u64 = 40;

/// A top-level listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `GET /actors`.
    Actors,
    /// `GET /messages`.
    Messages,
    /// `GET /outcomes`.
    Outcomes,
    /// `GET /turns`.
    Turns,
    /// `GET /nodes`.
    Nodes,
    /// `GET /types`.
    Types,
}

impl Kind {
    /// The command name, e.g. `actors`.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Actors => "actors",
            Kind::Messages => "messages",
            Kind::Outcomes => "outcomes",
            Kind::Turns => "turns",
            Kind::Nodes => "nodes",
            Kind::Types => "types",
        }
    }
}

/// The `:` commands, for completion.
pub const COMMANDS: [&str; 13] = [
    "actors", "actor", "mailbox", "messages", "message", "outcomes", "turns", "nodes", "node", "types", "type", "help",
    "quit",
];
const STATUSES: [&str; 4] = ["done", "failed", "cancelled", "purged"];

/// What a row stands for: what Enter opens and actions apply to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// An actor.
    Actor(ActorAddress),
    /// A message, pending or finished.
    Message(MessageId),
    /// The turn running on an actor.
    Turn(ActorAddress),
    /// A node.
    Node(NodeId),
    /// A registered type.
    Type(TypeName),
    /// The state schema (`state`) or a message schema of a type.
    Schema(String),
}

/// `actor x/y`, `message m1`, …
pub fn label(item: &Item) -> String {
    match item {
        Item::Actor(a) => format!("actor {a}"),
        Item::Message(id) => format!("message {id}"),
        Item::Turn(a) => format!("turn on actor {a}"),
        Item::Node(id) => format!("node {id}"),
        Item::Type(name) => format!("type {name}"),
        Item::Schema(name) => format!("schema {name}"),
    }
}

/// What a screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A listing under an address prefix; outcomes optionally of one status.
    List {
        /// Which listing.
        kind: Kind,
        /// Address prefix.
        prefix: String,
        /// Outcome status.
        status: Option<OutcomeStatus>,
    },
    /// The mailbox of an actor.
    Mailbox(ActorAddress),
    /// An actor: facts, state, mailbox, outcomes and children.
    Actor(ActorAddress),
    /// A message: facts, payload and the messages it caused.
    Message(MessageId),
    /// A node: facts and its turns.
    Node(NodeId),
    /// A type: facts and its schemas.
    Type(TypeName),
    /// One item as YAML or JSON.
    Describe(Item),
}

impl Target {
    /// A top-level listing without filters.
    pub fn list(kind: Kind) -> Self {
        Target::List { kind, prefix: String::new(), status: None }
    }

    /// The breadcrumb.
    pub fn crumb(&self) -> String {
        match self {
            Target::List { kind, .. } => kind.name().to_owned(),
            Target::Mailbox(_) => "mailbox".to_owned(),
            Target::Actor(a) => a.to_string(),
            Target::Message(id) => id.to_string(),
            Target::Node(id) => id.to_string(),
            Target::Type(name) => name.to_string(),
            Target::Describe(_) => "describe".to_owned(),
        }
    }

    /// The title of the main table or view, e.g. `outcomes(failed)` or `mailbox(account/acme:7)`.
    pub fn title(&self) -> String {
        match self {
            Target::List { kind, prefix, status } => {
                let scope: Vec<&str> = status
                    .map(|s| s.wire_name())
                    .into_iter()
                    .chain((!prefix.is_empty()).then_some(prefix.as_str()))
                    .collect();
                if scope.is_empty() {
                    format!("{}(all)", kind.name())
                } else {
                    format!("{}({})", kind.name(), scope.join(" "))
                }
            }
            Target::Mailbox(a) => format!("mailbox({a})"),
            Target::Describe(item) => label(item),
            other => other.crumb(),
        }
    }
}

/// One labelled value of an overview.
#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    /// The label.
    pub label: &'static str,
    /// The value.
    pub value: String,
    /// Its colour.
    pub tone: Tone,
}

/// The top half of an overview screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Overview {
    /// What the screen is about; actions without a fitting row apply to it.
    pub subject: Item,
    /// Labelled values.
    pub facts: Vec<Fact>,
    /// JSON panes, each with a title.
    pub panes: Vec<(String, Value)>,
    /// True when the pane shown is the one of the selected row (a type's schemas); else the first.
    pub pane_follows_selection: bool,
    /// The message whose turn sent this message, for `b`.
    pub caused_by: Option<MessageId>,
    /// The actor of this message, for `a`.
    pub actor: Option<ActorAddress>,
    /// The parent of this actor, for `P`.
    pub parent: Option<ActorAddress>,
}

/// The state of the describe view.
#[derive(Debug, Clone, PartialEq)]
pub struct Describe {
    /// What is shown.
    pub value: Value,
    /// YAML (`true`) or JSON.
    pub yaml: bool,
    /// Long lines wrap.
    pub wrap: bool,
    /// Lines scrolled down; the drawing code keeps it in range.
    pub scroll: u16,
}

/// What a fetch of a screen returned.
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenData {
    /// The rows of a listing.
    Grid(Rows),
    /// An overview and the rows of its tabs.
    Overview(Overview, Vec<Rows>),
    /// The value to describe.
    Describe(Value),
}

/// One screen of the stack.
#[derive(Debug, Clone)]
pub struct Screen {
    /// What it shows.
    pub target: Target,
    /// Its tables, each with a tab name; one unnamed for a listing, none for describe.
    pub tabs: Vec<(&'static str, Grid)>,
    /// The tab shown.
    pub tab: usize,
    /// The facts and panes of an overview.
    pub overview: Option<Overview>,
    /// The describe view.
    pub describe: Option<Describe>,
    /// The error of the last fetch, until one succeeds.
    pub error: Option<String>,
}

impl Screen {
    fn new(target: Target) -> Self {
        let names: &[&'static str] = match &target {
            Target::List { .. } | Target::Mailbox(_) => &[""],
            Target::Actor(_) => &["mailbox", "outcomes", "children"],
            Target::Message(_) => &["caused"],
            Target::Node(_) => &["turns"],
            Target::Type(_) => &["schemas"],
            Target::Describe(_) => &[],
        };
        let tabs = names.iter().map(|n| (*n, Grid::default())).collect();
        Self { target, tabs, tab: 0, overview: None, describe: None, error: None }
    }

    /// The table shown.
    pub fn grid(&self) -> Option<&Grid> {
        self.tabs.get(self.tab).map(|(_, g)| g)
    }

    /// The table shown, to change.
    pub fn grid_mut(&mut self) -> Option<&mut Grid> {
        self.tabs.get_mut(self.tab).map(|(_, g)| g)
    }

    /// True once data arrived.
    pub fn loaded(&self) -> bool {
        self.overview.is_some() || self.describe.is_some() || self.tabs.first().is_some_and(|(_, g)| g.loaded)
    }

    fn apply(&mut self, data: ScreenData) {
        self.error = None;
        match data {
            ScreenData::Grid(rows) => {
                if let Some((_, grid)) = self.tabs.first_mut() {
                    grid.set(rows);
                }
            }
            ScreenData::Overview(overview, rows) => {
                self.overview = Some(overview);
                for ((_, grid), rows) in self.tabs.iter_mut().zip(rows) {
                    grid.set(rows);
                }
            }
            ScreenData::Describe(value) => match &mut self.describe {
                Some(describe) => describe.value = value,
                None => self.describe = Some(Describe { value, yaml: true, wrap: false, scroll: 0 }),
            },
        }
    }
}

/// The header's view of the whole system.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// Actors and pending messages.
    pub stats: StatsView,
    /// Every node.
    pub nodes: Vec<NodeView>,
}

/// A change, run after the user confirms it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Delete an actor, or remove a type's registration.
    Delete(Item),
    /// Cancel a pending message.
    Cancel(MessageId),
    /// Stop the turn running on an actor.
    StopTurn(ActorAddress),
    /// Retry a finished message.
    Retry(MessageId),
    /// Pause an actor's mailbox.
    Pause(ActorAddress),
    /// Resume an actor's mailbox.
    Resume(ActorAddress),
    /// Purge an actor's mailbox.
    Purge(ActorAddress),
    /// Make a pending message due now.
    DeliverNow(MessageId),
}

impl Action {
    /// The question asked before running it.
    pub fn question(&self) -> String {
        match self {
            Action::Delete(Item::Type(name)) => format!("Remove the registration of type {name}?"),
            Action::Delete(item) => {
                format!("Delete {} with all its descendants and purge their mailboxes?", label(item))
            }
            Action::Cancel(id) => format!("Cancel message {id}?"),
            Action::StopTurn(actor) => format!("Stop the turn running on {actor}?"),
            Action::Retry(id) => format!("Send message {id} again as a new message?"),
            Action::Pause(actor) => format!("Pause actor {actor}?"),
            Action::Resume(actor) => format!("Resume actor {actor}?"),
            Action::Purge(actor) => format!("Purge every pending message of actor {actor}?"),
            Action::DeliverNow(id) => format!("Deliver message {id} now?"),
        }
    }

    /// True for actions that destroy something.
    pub fn dangerous(&self) -> bool {
        matches!(self, Action::Delete(_) | Action::Purge(_) | Action::StopTurn(_))
    }
}

/// Work for the event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Load the data of `target`; the answer carries `seq`.
    Fetch {
        /// Matches the answer to this request.
        seq: u64,
        /// What to load.
        target: Target,
    },
    /// Load the header summary.
    Summary,
    /// Run a confirmed action.
    Act(Action),
    /// Leave the TUI.
    Quit,
}

/// What came back from an [`Effect`].
#[derive(Debug, Clone, PartialEq)]
pub enum Fetched {
    /// The data of the fetch numbered `seq`.
    Screen {
        /// The fetch it answers.
        seq: u64,
        /// The data.
        data: ScreenData,
    },
    /// The header summary, or why it failed.
    Summary(Result<Summary, String>),
    /// An action succeeded.
    Done {
        /// The action.
        action: Action,
        /// What it did.
        text: String,
    },
    /// A request failed; `seq` is set for a screen fetch.
    Failed {
        /// The fetch it answers, if it was one.
        seq: Option<u64>,
        /// What went wrong.
        error: String,
    },
}

/// What keys do right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Move, open, act.
    Normal,
    /// Typing a `:` command.
    Command(String),
    /// Typing the live filter of the shown table.
    Filter,
    /// The confirmation dialog; `yes` is the focused button.
    Confirm {
        /// What runs on yes.
        action: Action,
        /// True when Yes has the focus.
        yes: bool,
    },
    /// The key list is shown.
    Help,
}

/// A message at the bottom right that goes away by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    /// The text.
    pub text: String,
    /// Red rather than green.
    pub error: bool,
    /// The tick it disappears at.
    pub until: u64,
}

/// Where the shown table is on screen, for the mouse; set by the drawing code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hit {
    /// The table without its border; the first line is the header.
    pub area: Rect,
    /// The x range of each column.
    pub columns: Vec<(u16, u16)>,
}

/// The whole TUI state.
#[derive(Debug)]
pub struct App {
    /// The context name, if any.
    pub context: Option<String>,
    /// The server URL.
    pub server: String,
    /// The screens; the last one is shown.
    pub stack: Vec<Screen>,
    /// What keys do.
    pub mode: Mode,
    /// Show the wide columns.
    pub wide: bool,
    /// The last header summary.
    pub summary: Option<Result<Summary, String>>,
    /// The current toast.
    pub toast: Option<Toast>,
    /// Ticks so far.
    pub tick: u64,
    /// Where the table is, for the mouse.
    pub hit: Option<Hit>,
    seq: u64,
    in_flight: Option<u64>,
    summary_in_flight: bool,
}

impl App {
    /// A TUI on the actors listing; call [`Self::start`] for the first fetches.
    pub fn new(context: Option<String>, server: String) -> Self {
        Self {
            context,
            server,
            stack: vec![Screen::new(Target::list(Kind::Actors))],
            mode: Mode::Normal,
            wide: false,
            summary: None,
            toast: None,
            tick: 0,
            hit: None,
            seq: 0,
            in_flight: None,
            summary_in_flight: false,
        }
    }

    /// The first fetches.
    pub fn start(&mut self) -> Vec<Effect> {
        let mut effects = self.fetch();
        effects.extend(self.summarize());
        effects
    }

    /// The screen shown.
    pub fn screen(&self) -> &Screen {
        self.stack.last().expect("the stack is never empty")
    }

    fn screen_mut(&mut self) -> &mut Screen {
        self.stack.last_mut().expect("the stack is never empty")
    }

    /// True while a fetch of the shown screen is running.
    pub fn loading(&self) -> bool {
        self.in_flight.is_some()
    }

    /// The suggested completion of a `:` command line, if any.
    pub fn completion(text: &str) -> Option<String> {
        let (word, rest) = text.split_once(' ').unwrap_or((text, ""));
        if text.is_empty() {
            return None;
        }
        if !text.contains(' ') {
            return COMMANDS.iter().find(|c| c.starts_with(word) && **c != word).map(|c| c.to_string());
        }
        if matches!(word, "outcomes" | "o") && !rest.contains(' ') {
            return STATUSES.iter().find(|s| s.starts_with(rest) && **s != rest).map(|s| format!("{word} {s}"));
        }
        None
    }

    fn fetch(&mut self) -> Vec<Effect> {
        self.seq += 1;
        self.in_flight = Some(self.seq);
        vec![Effect::Fetch { seq: self.seq, target: self.screen().target.clone() }]
    }

    fn summarize(&mut self) -> Vec<Effect> {
        if self.summary_in_flight {
            return vec![];
        }
        self.summary_in_flight = true;
        vec![Effect::Summary]
    }

    fn push(&mut self, target: Target) -> Vec<Effect> {
        self.stack.push(Screen::new(target));
        self.fetch()
    }

    fn pop(&mut self) -> Vec<Effect> {
        if self.stack.len() == 1 {
            return vec![];
        }
        self.stack.pop();
        self.fetch()
    }

    fn root(&mut self, target: Target) -> Vec<Effect> {
        self.stack = vec![Screen::new(target)];
        self.fetch()
    }

    fn toast(&mut self, text: impl Into<String>, error: bool) {
        let ticks = if error { ERROR_TICKS } else { TOAST_TICKS };
        self.toast = Some(Toast { text: text.into(), error, until: self.tick + ticks });
    }

    /// A tick of 250 ms: toasts expire, screen and summary refresh on their schedule.
    pub fn on_tick(&mut self) -> Vec<Effect> {
        self.tick += 1;
        if self.toast.as_ref().is_some_and(|t| t.until <= self.tick) {
            self.toast = None;
        }
        let mut effects = Vec::new();
        if self.tick.is_multiple_of(TICKS_PER_REFRESH) && !self.loading() {
            effects.extend(self.fetch());
        }
        if self.tick.is_multiple_of(TICKS_PER_SUMMARY) {
            effects.extend(self.summarize());
        }
        effects
    }

    /// Handles a key press.
    pub fn on_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if ctrl(key, 'c') {
            return vec![Effect::Quit];
        }
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Command(text) => self.on_command_key(key, text),
            Mode::Filter => self.on_filter_key(key),
            Mode::Confirm { action, yes } => self.on_confirm_key(key, action, yes),
            Mode::Help => vec![],
            Mode::Normal if self.screen().describe.is_some() => self.on_describe_key(key),
            Mode::Normal => self.on_normal_key(key),
        }
    }

    fn on_command_key(&mut self, key: KeyEvent, mut text: String) -> Vec<Effect> {
        match key.code {
            KeyCode::Enter => return self.run_command(&text),
            KeyCode::Esc => return vec![],
            KeyCode::Tab | KeyCode::Right => text = Self::completion(&text).unwrap_or(text),
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char(c) => text.push(c),
            _ => {}
        }
        self.mode = Mode::Command(text);
        vec![]
    }

    fn run_command(&mut self, text: &str) -> Vec<Effect> {
        let words: Vec<&str> = text.split_whitespace().collect();
        let (command, args) = words.split_first().map(|(c, a)| (*c, a)).unwrap_or(("", &[]));
        let arg = args.first().copied().unwrap_or_default();
        let list = |kind| Target::List { kind, prefix: arg.to_owned(), status: None };
        let target = match command {
            "" => return vec![],
            "q" | "quit" | "q!" => return vec![Effect::Quit],
            "help" | "h" | "?" => {
                self.mode = Mode::Help;
                return vec![];
            }
            "actors" | "a" => list(Kind::Actors),
            "messages" | "msgs" | "m" => list(Kind::Messages),
            "turns" | "t" => list(Kind::Turns),
            "nodes" | "no" => list(Kind::Nodes),
            "types" | "ty" => list(Kind::Types),
            "outcomes" | "o" => match OutcomeStatus::from_wire_name(arg) {
                Some(status) => Target::List {
                    kind: Kind::Outcomes,
                    prefix: args.get(1).copied().unwrap_or_default().to_owned(),
                    status: Some(status),
                },
                None => list(Kind::Outcomes),
            },
            "actor" | "mailbox" | "mb" => match ActorAddress::parse(arg) {
                Ok(a) if command == "actor" => Target::Actor(a),
                Ok(a) => Target::Mailbox(a),
                Err(e) => return self.refuse(e.to_string()),
            },
            "message" | "msg" => match MessageId::new(arg) {
                Ok(id) => Target::Message(id),
                Err(e) => return self.refuse(e.to_string()),
            },
            "node" => match NodeId::new(arg) {
                Ok(id) => Target::Node(id),
                Err(e) => return self.refuse(e.to_string()),
            },
            "type" => match TypeName::parse(arg) {
                Ok(name) => Target::Type(name),
                Err(e) => return self.refuse(e.to_string()),
            },
            other => return self.refuse(format!("unknown command ':{other}'; ? lists them")),
        };
        self.root(target)
    }

    fn refuse(&mut self, error: impl Into<String>) -> Vec<Effect> {
        self.toast(error, true);
        vec![]
    }

    fn on_filter_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(grid) = self.screen_mut().grid_mut() else { return vec![] };
        match key.code {
            KeyCode::Enter => return vec![],
            KeyCode::Esc => {
                grid.set_filter(String::new());
                return vec![];
            }
            KeyCode::Backspace => {
                let mut filter = grid.filter.clone();
                filter.pop();
                grid.set_filter(filter);
            }
            KeyCode::Char(c) => grid.set_filter(format!("{}{c}", grid.filter)),
            _ => {}
        }
        self.mode = Mode::Filter;
        vec![]
    }

    fn on_confirm_key(&mut self, key: KeyEvent, action: Action, yes: bool) -> Vec<Effect> {
        match key.code {
            KeyCode::Char('y') => vec![Effect::Act(action)],
            KeyCode::Enter if yes => vec![Effect::Act(action)],
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('h' | 'l') => {
                self.mode = Mode::Confirm { action, yes: !yes };
                vec![]
            }
            _ => vec![],
        }
    }

    fn on_describe_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if let Some(effects) = self.on_common_key(key) {
            return effects;
        }
        let describe = self.screen_mut().describe.as_mut().expect("a describe screen");
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => describe.scroll = describe.scroll.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => describe.scroll = describe.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => describe.scroll = describe.scroll.saturating_add(20),
            KeyCode::PageUp => describe.scroll = describe.scroll.saturating_sub(20),
            KeyCode::Home | KeyCode::Char('g') => describe.scroll = 0,
            KeyCode::End | KeyCode::Char('G') => describe.scroll = u16::MAX,
            KeyCode::Char('t') => describe.yaml = !describe.yaml,
            KeyCode::Char('w') => describe.wrap = !describe.wrap,
            _ => {}
        }
        vec![]
    }

    /// Keys that work on every screen; `None` when `key` is not one of them.
    fn on_common_key(&mut self, key: KeyEvent) -> Option<Vec<Effect>> {
        Some(match key.code {
            _ if ctrl(key, 'r') => {
                let mut effects = self.fetch();
                effects.extend(self.summarize());
                effects
            }
            KeyCode::Char('q') if self.stack.len() == 1 => vec![Effect::Quit],
            KeyCode::Char('q') => self.pop(),
            KeyCode::Char(':') => {
                self.mode = Mode::Command(String::new());
                vec![]
            }
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                vec![]
            }
            KeyCode::Esc => match self.screen_mut().grid_mut() {
                Some(grid) if !grid.filter.is_empty() => {
                    grid.set_filter(String::new());
                    vec![]
                }
                _ => self.pop(),
            },
            _ => return None,
        })
    }

    fn on_normal_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if let Some(effects) = self.on_common_key(key) {
            return effects;
        }
        // ctrl-d and ctrl-k carry the codes of `d` and `k`; they are actions, never describe or move.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if let Some(action) = self.action(key) {
                self.mode = Mode::Confirm { action, yes: false };
            }
            return vec![];
        }
        let wide = self.wide;
        let screen = self.screen_mut();
        let tabs = screen.tabs.len();
        if let Some(grid) = screen.grid_mut() {
            let columns = if wide { grid.data.wide_headers.len() } else { grid.data.headers.len() };
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => grid.step(1),
                KeyCode::Up | KeyCode::Char('k') => grid.step(-1),
                KeyCode::PageDown => grid.step(10),
                KeyCode::PageUp => grid.step(-10),
                KeyCode::Home | KeyCode::Char('g') => grid.jump(false),
                KeyCode::End | KeyCode::Char('G') => grid.jump(true),
                KeyCode::Char('<') => grid.sort_step(-1, columns),
                KeyCode::Char('>') => grid.sort_step(1, columns),
                KeyCode::Char('s') => grid.flip(),
                _ => return self.on_screen_key(key, tabs),
            }
            return vec![];
        }
        self.on_screen_key(key, tabs)
    }

    fn on_screen_key(&mut self, key: KeyEvent, tabs: usize) -> Vec<Effect> {
        let overview = self.screen().overview.clone();
        match key.code {
            KeyCode::Char('/') if self.screen().grid().is_some() => self.mode = Mode::Filter,
            KeyCode::Tab if tabs > 1 => self.screen_mut().tab = (self.screen().tab + 1) % tabs,
            KeyCode::BackTab if tabs > 1 => self.screen_mut().tab = (self.screen().tab + tabs - 1) % tabs,
            KeyCode::Char('w') => self.wide = !self.wide,
            KeyCode::Enter => return self.drill(),
            KeyCode::Char('d') => {
                let item = self.selected().filter(|i| !matches!(i, Item::Schema(_))).or(overview.map(|o| o.subject));
                return item.map(|i| self.push(Target::Describe(i))).unwrap_or_default();
            }
            KeyCode::Char('m') => {
                let actor = match (self.selected(), overview.map(|o| o.subject)) {
                    (Some(Item::Actor(a)), _) | (_, Some(Item::Actor(a))) => a,
                    _ => return vec![],
                };
                return self.push(Target::Mailbox(actor));
            }
            KeyCode::Char('a') => {
                if let Some(actor) = overview.and_then(|o| o.actor) {
                    return self.push(Target::Actor(actor));
                }
            }
            KeyCode::Char('b') => {
                if let Some(cause) = overview.and_then(|o| o.caused_by) {
                    return self.push(Target::Message(cause));
                }
            }
            KeyCode::Char('P') => {
                if let Some(parent) = overview.and_then(|o| o.parent) {
                    return self.push(Target::Actor(parent));
                }
            }
            _ => {
                if let Some(action) = self.action(key) {
                    self.mode = Mode::Confirm { action, yes: false };
                }
            }
        }
        vec![]
    }

    fn selected(&self) -> Option<Item> {
        self.screen().grid().and_then(Grid::selected_item).cloned()
    }

    fn drill(&mut self) -> Vec<Effect> {
        let target = match self.selected() {
            Some(Item::Actor(a) | Item::Turn(a)) => Target::Actor(a),
            Some(Item::Message(id)) => Target::Message(id),
            Some(Item::Node(id)) => Target::Node(id),
            Some(Item::Type(name)) => Target::Type(name),
            Some(Item::Schema(_)) | None => return vec![],
        };
        self.push(target)
    }

    /// The action `key` stands for here, if it applies: to the selected row, else to the screen's subject. On a
    /// message screen, message actions apply to the message itself, not to the messages it caused.
    fn action(&self, key: KeyEvent) -> Option<Action> {
        let screen = self.screen();
        let row = self.selected();
        let subject = screen.overview.as_ref().map(|o| o.subject.clone());
        let actor = match (&row, &subject, &screen.target) {
            (Some(Item::Actor(a)), ..) | (_, Some(Item::Actor(a)), _) | (.., Target::Mailbox(a)) => Some(a.clone()),
            _ => None,
        };
        // The message and whether it is known to be finished (`None`: unknown, the server decides).
        let message = match (&screen.target, &row) {
            (Target::Message(id), _) => Some((id.clone(), None)),
            (_, Some(Item::Message(id))) => Some((id.clone(), screen.grid().map(|g| g.data.finished))),
            _ => None,
        };
        let pending = message.clone().filter(|(_, f)| *f != Some(true)).map(|(id, _)| id);
        let finished = message.filter(|(_, f)| *f != Some(false)).map(|(id, _)| id);
        if ctrl(key, 'd') {
            return [row, subject]
                .into_iter()
                .flatten()
                .find(|i| matches!(i, Item::Actor(_) | Item::Type(_)))
                .map(Action::Delete);
        }
        if ctrl(key, 'k') {
            return match row {
                Some(Item::Turn(a)) => Some(Action::StopTurn(a)),
                _ => actor.map(Action::StopTurn),
            };
        }
        match key.code {
            KeyCode::Char('c') => pending.map(Action::Cancel),
            KeyCode::Char('n') => pending.map(Action::DeliverNow),
            KeyCode::Char('r') => finished.map(Action::Retry),
            KeyCode::Char('p') => actor.map(Action::Pause),
            KeyCode::Char('u') => actor.map(Action::Resume),
            KeyCode::Char('x') => actor.map(Action::Purge),
            _ => None,
        }
    }

    /// Handles the mouse: the wheel moves or scrolls, a click selects a row, a click on the selected row opens it, a
    /// click on a column header sorts by it.
    pub fn on_mouse(&mut self, event: MouseEvent) -> Vec<Effect> {
        if self.mode != Mode::Normal {
            return vec![];
        }
        let delta = match event.kind {
            MouseEventKind::ScrollDown => 1,
            MouseEventKind::ScrollUp => -1,
            MouseEventKind::Down(MouseButton::Left) => return self.click(event.column, event.row),
            _ => return vec![],
        };
        let screen = self.screen_mut();
        if let Some(describe) = &mut screen.describe {
            describe.scroll = describe.scroll.saturating_add_signed(delta as i16 * 3);
        } else if let Some(grid) = screen.grid_mut() {
            grid.step(delta);
        }
        vec![]
    }

    fn click(&mut self, x: u16, y: u16) -> Vec<Effect> {
        let Some(hit) = self.hit.clone() else { return vec![] };
        let inside = x >= hit.area.x && x < hit.area.right() && y >= hit.area.y && y < hit.area.bottom();
        let Some(grid) = self.screen_mut().grid_mut().filter(|_| inside) else { return vec![] };
        if y == hit.area.y {
            if let Some(column) = hit.columns.iter().position(|&(from, to)| x >= from && x < to) {
                grid.sort_by(column);
            }
            return vec![];
        }
        let position = grid.offset + usize::from(y - hit.area.y - 1);
        if position >= grid.shown().len() {
            return vec![];
        }
        if position == grid.selected {
            return self.drill();
        }
        grid.select(position);
        vec![]
    }

    /// Takes what an effect returned; may ask for more.
    pub fn on_result(&mut self, result: Fetched) -> Vec<Effect> {
        match result {
            Fetched::Screen { seq, data } if self.in_flight == Some(seq) => {
                self.in_flight = None;
                self.screen_mut().apply(data);
            }
            Fetched::Failed { seq: Some(seq), error } if self.in_flight == Some(seq) => {
                self.in_flight = None;
                self.screen_mut().error = Some(error);
            }
            Fetched::Screen { .. } | Fetched::Failed { seq: Some(_), .. } => {}
            Fetched::Failed { seq: None, error } => self.toast(error, true),
            Fetched::Summary(summary) => {
                self.summary_in_flight = false;
                self.summary = Some(summary);
            }
            Fetched::Done { action, text } => {
                self.toast(text, false);
                let subject = self.screen().overview.as_ref().map(|o| &o.subject);
                let gone = matches!(&action, Action::Delete(item) if Some(item) == subject);
                let mut effects = if gone { self.pop() } else { self.fetch() };
                effects.extend(self.summarize());
                return effects;
            }
        }
        vec![]
    }
}

fn ctrl(key: KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::grid::GridRow;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn ctrl_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn typed(app: &mut App, text: &str) -> Vec<Effect> {
        text.chars().flat_map(|c| app.on_key(key(KeyCode::Char(c)))).collect()
    }

    fn message(id: &str) -> Item {
        Item::Message(MessageId::new(id).unwrap())
    }

    fn actor(text: &str) -> ActorAddress {
        ActorAddress::parse(text).unwrap()
    }

    fn rows(items: Vec<Item>, finished: bool) -> Rows {
        let rows = items
            .into_iter()
            .map(|item| {
                let cells = vec![label(&item)];
                GridRow { item, wide: cells.clone(), cells }
            })
            .collect();
        Rows { headers: vec!["WHAT"], wide_headers: vec!["WHAT"], rows, finished }
    }

    fn seq_of(effects: &[Effect]) -> u64 {
        effects.iter().find_map(|e| if let Effect::Fetch { seq, .. } = e { Some(*seq) } else { None }).expect("a fetch")
    }

    /// An app on `target` (the only screen) with `data` loaded.
    fn showing(target: Target, data: ScreenData) -> App {
        let mut app = App::new(None, "http://x".into());
        app.start();
        let seq = seq_of(&app.root(target));
        app.on_result(Fetched::Screen { seq, data });
        app
    }

    /// An app on the actors listing with the overview of `account/acme:7` pushed on it.
    fn actor_overview() -> App {
        let mut app = showing(Target::list(Kind::Actors), ScreenData::Grid(rows(vec![], false)));
        let overview = Overview {
            subject: Item::Actor(actor("account/acme:7")),
            facts: vec![],
            panes: vec![],
            pane_follows_selection: false,
            caused_by: None,
            actor: None,
            parent: None,
        };
        let seq = seq_of(&app.push(Target::Actor(actor("account/acme:7"))));
        let tabs = vec![rows(vec![message("p1")], false), rows(vec![message("f1")], true)];
        app.on_result(Fetched::Screen { seq, data: ScreenData::Overview(overview, tabs) });
        app
    }

    #[test]
    fn commands_complete_and_replace_the_stack() {
        assert_eq!(App::completion("out"), Some("outcomes".into()));
        assert_eq!(App::completion("outcomes fa"), Some("outcomes failed".into()));
        assert_eq!(App::completion("actors"), None);

        let mut app = App::new(None, "http://x".into());
        app.push(Target::Actor(actor("a/b")));
        typed(&mut app, ":o");
        app.on_key(key(KeyCode::Char(' ')));
        typed(&mut app, "f");
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.mode, Mode::Command("o failed".into()));
        typed(&mut app, " acc");
        let effects = app.on_key(key(KeyCode::Enter));
        let expected = Target::List { kind: Kind::Outcomes, prefix: "acc".into(), status: Some(OutcomeStatus::Failed) };
        assert!(matches!(&effects[..], [Effect::Fetch { target, .. }] if *target == expected));
        assert_eq!(app.stack.len(), 1);
        assert_eq!(app.screen().target.title(), "outcomes(failed acc)");

        typed(&mut app, ":nope");
        assert!(app.on_key(key(KeyCode::Enter)).is_empty());
        assert!(app.toast.as_ref().unwrap().error);
    }

    #[test]
    fn the_filter_is_live_and_esc_clears_it() {
        let mut app =
            showing(Target::list(Kind::Messages), ScreenData::Grid(rows(vec![message("a1"), message("b1")], false)));
        typed(&mut app, "/b");
        assert_eq!(app.mode, Mode::Filter);
        assert_eq!(app.screen().grid().unwrap().shown().len(), 1);
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.screen().grid().unwrap().filter, "b", "Enter keeps the filter");
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.screen().grid().unwrap().shown().len(), 2);
    }

    #[test]
    fn an_action_needs_yes_and_no_is_the_default() {
        let mut app = showing(Target::list(Kind::Messages), ScreenData::Grid(rows(vec![message("m1")], false)));
        app.on_key(key(KeyCode::Char('c')));
        let cancel = Action::Cancel(MessageId::new("m1").unwrap());
        assert_eq!(app.mode, Mode::Confirm { action: cancel.clone(), yes: false });
        assert!(app.on_key(key(KeyCode::Enter)).is_empty(), "Enter on No does nothing");
        for no in [KeyCode::Char('n'), KeyCode::Esc] {
            app.on_key(key(KeyCode::Char('c')));
            assert!(app.on_key(key(no)).is_empty(), "{no:?} cancels nothing");
            assert_eq!(app.mode, Mode::Normal, "{no:?} closes the dialog");
        }

        app.on_key(key(KeyCode::Char('c')));
        app.on_key(key(KeyCode::Left));
        assert_eq!(app.on_key(key(KeyCode::Enter)), vec![Effect::Act(cancel.clone())]);
        app.on_key(key(KeyCode::Char('c')));
        assert_eq!(app.on_key(key(KeyCode::Char('y'))), vec![Effect::Act(cancel)]);
    }

    #[test]
    fn actions_pick_the_row_or_the_subject() {
        let mut app = actor_overview();
        let acme = actor("account/acme:7");
        app.on_key(ctrl_key('d'));
        assert_eq!(app.mode, Mode::Confirm { action: Action::Delete(Item::Actor(acme.clone())), yes: false });
        app.mode = Mode::Normal;
        app.on_key(key(KeyCode::Char('c')));
        assert_eq!(app.mode, Mode::Confirm { action: Action::Cancel(MessageId::new("p1").unwrap()), yes: false });
        app.mode = Mode::Normal;
        app.on_key(key(KeyCode::Char('r')));
        assert_eq!(app.mode, Mode::Normal, "a pending message cannot be retried");
        app.on_key(key(KeyCode::Tab));
        app.on_key(key(KeyCode::Char('r')));
        assert_eq!(app.mode, Mode::Confirm { action: Action::Retry(MessageId::new("f1").unwrap()), yes: false });
        app.mode = Mode::Normal;
        app.on_key(ctrl_key('k'));
        assert_eq!(app.mode, Mode::Confirm { action: Action::StopTurn(acme), yes: false });
    }

    #[test]
    fn enter_drills_down_and_esc_walks_back() {
        let mut app = showing(Target::list(Kind::Turns), ScreenData::Grid(rows(vec![Item::Turn(actor("a/b"))], false)));
        let effects = app.on_key(key(KeyCode::Enter));
        assert!(matches!(&effects[..], [Effect::Fetch { target: Target::Actor(a), .. }] if *a == actor("a/b")));
        typed(&mut app, "d");
        assert!(matches!(app.screen().target, Target::Actor(_)), "nothing to describe before the data is there");
        assert_eq!(app.stack.iter().map(|s| s.target.crumb()).collect::<Vec<_>>(), ["turns", "a/b"]);
        let effects = app.on_key(key(KeyCode::Esc));
        assert!(matches!(&effects[..], [Effect::Fetch { target: Target::List { kind: Kind::Turns, .. }, .. }]));
        assert!(app.screen().loaded(), "the screen we return to keeps its rows while it refreshes");
    }

    #[test]
    fn deleting_the_subject_goes_back() {
        let mut app = actor_overview();
        let action = Action::Delete(Item::Actor(actor("account/acme:7")));
        let effects = app.on_result(Fetched::Done { action, text: "deleted".into() });
        assert_eq!(app.stack.len(), 1);
        assert!(matches!(&effects[0], Effect::Fetch { target: Target::List { kind: Kind::Actors, .. }, .. }));
        assert_eq!(app.toast.as_ref().unwrap().text, "deleted");
    }

    #[test]
    fn a_late_answer_for_another_screen_is_dropped() {
        let mut app = App::new(None, "http://x".into());
        let old = seq_of(&app.start());
        typed(&mut app, ":types");
        let new = seq_of(&app.on_key(key(KeyCode::Enter)));
        app.on_result(Fetched::Screen { seq: old, data: ScreenData::Grid(rows(vec![message("stale")], false)) });
        assert!(!app.screen().loaded());
        app.on_result(Fetched::Screen {
            seq: new,
            data: ScreenData::Grid(rows(vec![Item::Type(TypeName::parse("t").unwrap())], false)),
        });
        assert!(app.screen().loaded() && !app.loading());
    }

    #[test]
    fn ticks_refresh_on_schedule_and_expire_toasts() {
        let mut app = App::new(None, "http://x".into());
        let seq = seq_of(&app.start());
        app.on_result(Fetched::Screen { seq, data: ScreenData::Grid(rows(vec![], false)) });
        app.on_result(Fetched::Summary(Err("down".into())));
        app.toast("hi", false);
        let effects: Vec<Effect> = (0..TICKS_PER_SUMMARY).flat_map(|_| app.on_tick()).collect();
        assert!(effects.iter().filter(|e| matches!(e, Effect::Fetch { .. })).count() >= 1);
        assert!(effects.contains(&Effect::Summary));
        assert!(app.toast.is_none());
        assert!(app.on_tick().is_empty(), "no new fetch while one is running");
    }

    #[test]
    fn the_mouse_selects_opens_and_sorts() {
        let mut app =
            showing(Target::list(Kind::Messages), ScreenData::Grid(rows(vec![message("a"), message("b")], false)));
        app.hit = Some(Hit { area: Rect::new(1, 1, 40, 10), columns: vec![(1, 20)] });
        let click = |x, y| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        };
        assert!(app.on_mouse(click(5, 3)).is_empty());
        assert_eq!(app.screen().grid().unwrap().selected, 1);
        let effects = app.on_mouse(click(5, 3));
        assert!(matches!(&effects[..], [Effect::Fetch { target: Target::Message(_), .. }]));
        app.on_key(key(KeyCode::Esc));
        app.on_mouse(click(5, 1));
        assert_eq!(app.screen().grid().unwrap().sort, Some((0, false)));
    }
}
