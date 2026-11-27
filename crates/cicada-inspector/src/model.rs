//! The portable model: what the inspector fetches over the HTTP API, how a refresh is bounded, which context it shows,
//! and the sidebar's history. The AppKit view only renders it.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cicada::ActorAddress;
use cicada::api::{NodeView, RunningTurnView, StatsView};
use cicada::store::MessageId;
use cicada::time::Timestamp;
use cicada_app::{Contexts, Env};
use cicada_client::{ActorClient, ClientError};
use futures::{StreamExt, stream};
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout_at};

use crate::actors::Actors;
use crate::messages::Messages;
use crate::types::Types;
use crate::watches::{WatchPlace, Watches};

/// What the inspector says where it cannot run.
pub const NEEDS_MACOS: &str = "the Cicada inspector requires macOS 14 or later";
/// The running turns the Overview shows: the first page of `/turns`.
pub const TURNS_PAGE: usize = 20;
/// The text of a part whose call had not answered at the deadline.
pub const TIMED_OUT: &str = "no answer within 10 s";
/// The variables that win over the picked context, as they do for `cicada`.
const OVERRIDES: [&str; 2] = ["CICADA_SERVER", "CICADA_TOKEN"];

/// Refuses a macOS before 14, as the tray does.
pub fn check_macos(version: (isize, isize, isize)) -> Result<(), &'static str> {
    if version < (14, 0, 0) { Err(NEEDS_MACOS) } else { Ok(()) }
}

/// The bounds of one refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most types whose actors are counted.
    pub types: usize,
    /// The most per-type counts in flight at once.
    pub concurrent: usize,
    /// How long the whole refresh may take; what has not answered by then is not shown. Each read of the Actors page
    /// has as long.
    pub deadline: Duration,
    /// How long the search field must be still before its prefix is read.
    pub debounce: Duration,
}

impl Limits {
    /// 50 types, 8 at a time, 10 seconds; a search waits 300 ms.
    pub const DEFAULT: Self =
        Self { types: 50, concurrent: 8, deadline: Duration::from_secs(10), debounce: Duration::from_millis(300) };
}

/// One part of the Overview: its value, or why it has none.
#[derive(Debug, Clone, PartialEq)]
pub enum Part<T> {
    /// The call answered.
    Ready(T),
    /// The call failed or did not answer by the deadline: the text to show, without credentials.
    Failed(String),
}

impl<T> Part<T> {
    /// The value mapped by `f`, or the same failure.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Part<U> {
        match self {
            Self::Ready(value) => Part::Ready(f(value)),
            Self::Failed(error) => Part::Failed(error),
        }
    }

    /// The value, if the call answered.
    #[cfg(test)]
    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            Self::Failed(_) => None,
        }
    }
}

/// The actor count of one type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Count {
    /// `/stats?prefix=<type>/` answered.
    Counted(u64),
    /// The count's call failed: the text to show.
    Failed(String),
    /// Not asked: the type is past the most types counted, or the deadline came first.
    NotCounted,
}

/// A registered type and its actor count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRow {
    /// The type's name.
    pub name: String,
    /// How many actors it has.
    pub actors: Count,
}

/// The first page of the running turns.
#[derive(Debug, Clone, PartialEq)]
pub struct Turns {
    /// At most [`TURNS_PAGE`] turns.
    pub items: Vec<RunningTurnView>,
    /// True when the server has more than this page.
    pub more: bool,
}

/// What the Overview shows, each part with its own result.
#[derive(Debug, Clone, PartialEq)]
pub struct Overview {
    /// `/stats` of the whole store.
    pub stats: Part<StatsView>,
    /// `/health`: true when the node and its store answer.
    pub health: Part<bool>,
    /// `/nodes`.
    pub nodes: Part<Vec<NodeView>>,
    /// The first page of `/turns`.
    pub turns: Part<Turns>,
    /// `/types`, with each type's actor count.
    pub types: Part<Vec<TypeRow>>,
}

/// Gathers an Overview within `limits`: the base routes at once, and once `/types` answers, the actor counts of the
/// first types, a bounded number at a time. At the deadline the answers that came are kept; a base route still out
/// fails with [`TIMED_OUT`], and a type still uncounted is [`Count::NotCounted`].
pub async fn overview(client: &ActorClient, limits: Limits) -> Overview {
    let deadline = Instant::now() + limits.deadline;
    let turns = async {
        let page = client.turns(None, "", None, Some(TURNS_PAGE)).await?;
        Ok(Turns { more: page.next.is_some(), items: page.items })
    };
    // The counts start as soon as `/types` answers, not after the other base routes.
    let types = async {
        match within(deadline, client.types()).await {
            Part::Ready(types) => {
                Part::Ready(count(client, types.into_iter().map(|t| t.name).collect(), deadline, limits).await)
            }
            Part::Failed(error) => Part::Failed(error),
        }
    };
    let (stats, health, nodes, turns, types) = tokio::join!(
        within(deadline, client.stats("")),
        within(deadline, client.health()),
        within(deadline, client.nodes()),
        within(deadline, turns),
        types,
    );
    Overview { stats, health, nodes, turns, types }
}

/// The result of `call`, or [`TIMED_OUT`] when it has not answered by `deadline`.
pub(crate) async fn within<T>(deadline: Instant, call: impl Future<Output = Result<T, ClientError>>) -> Part<T> {
    match timeout_at(deadline, call).await {
        Ok(Ok(value)) => Part::Ready(value),
        Ok(Err(error)) => Part::Failed(error_text(&error)),
        Err(_) => Part::Failed(TIMED_OUT.into()),
    }
}

/// The actor counts of the first `limits.types` of `names`, in their order, at most `limits.concurrent` at a time.
async fn count(client: &ActorClient, names: Vec<String>, deadline: Instant, limits: Limits) -> Vec<TypeRow> {
    let stats = |name: String| async move { client.stats(&format!("{name}/")).await.map(|stats| stats.actors) };
    count_with(names, deadline, limits, stats).await
}

/// [`count`] with the call of one count given: `count_one` is called only for a count that starts before the
/// deadline, so a count still queued then never starts; one that runs at the deadline is dropped, uncounted.
pub(crate) async fn count_with<F, Fut>(
    names: Vec<String>,
    deadline: Instant,
    limits: Limits,
    count_one: F,
) -> Vec<TypeRow>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<u64, ClientError>>,
{
    let asked = names.len().min(limits.types);
    let count_one = &count_one;
    let counts: Vec<Count> = stream::iter(names[..asked].iter().cloned().map(|name| async move {
        if Instant::now() >= deadline {
            return Count::NotCounted;
        }
        match timeout_at(deadline, count_one(name)).await {
            Ok(Ok(actors)) => Count::Counted(actors),
            Ok(Err(error)) => Count::Failed(error_text(&error)),
            Err(_) => Count::NotCounted,
        }
    }))
    .buffered(limits.concurrent.max(1))
    .collect()
    .await;
    let counts = counts.into_iter().chain(std::iter::repeat(Count::NotCounted));
    names.into_iter().zip(counts).map(|(name, actors)| TypeRow { name, actors }).collect()
}

/// `error` and its causes, each only when the text so far does not hold it. It holds no credentials of the server
/// URL: the client never has them ([`cicada_app::context_client`]).
pub fn error_text(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text = format!("{text}: {cause_text}");
        }
        source = cause.source();
    }
    text
}

/// What the window shows: the contexts, the target in use and the latest Overview.
#[derive(Debug, Clone, Default)]
pub struct View {
    /// The contexts of the config file.
    pub contexts: Contexts,
    /// Why the contexts could not be read.
    pub contexts_error: Option<String>,
    /// The context the user picked; `None` until they pick one.
    pub picked: Option<String>,
    /// The context in use, as `cicada` resolves it.
    pub context: Option<String>,
    /// The variables set in the environment that win over the picked context.
    pub overrides: Vec<&'static str>,
    /// Why no client could be made for the context; the window stays usable.
    pub target_error: Option<String>,
    /// The latest Overview of the context in use, if one finished.
    pub overview: Option<Overview>,
    /// True while a refresh runs.
    pub refreshing: bool,
}

impl View {
    /// The subtitle's target, including a server configured without a named context.
    pub fn target_label(&self) -> &str {
        self.context.as_deref().or(self.picked.as_deref()).unwrap_or(if self.target_error.is_some() {
            "No context"
        } else {
            "Server"
        })
    }
}

/// The context in use and its refreshes, publishing each [`View`] through a `watch`. A new refresh, or a switch of
/// context, cancels the one before; a refresh that still finishes after that is discarded.
pub struct Session {
    env: Env,
    limits: Limits,
    handle: Handle,
    shared: Arc<Shared>,
    actors: Actors,
    messages: Messages,
    types: Types,
    watches: Watches,
}

struct Shared {
    view: watch::Sender<View>,
    current: Mutex<Current>,
}

#[derive(Default)]
struct Current {
    /// Grows with every refresh and every switch of context; a refresh publishes only while it is its own.
    generation: u64,
    client: Option<ActorClient>,
    task: Option<JoinHandle<()>>,
    /// The refreshes cancelled so far, for tests that await their end.
    #[cfg(test)]
    superseded: Vec<JoinHandle<()>>,
}

impl Session {
    /// A session with no context yet; [`Self::use_context`] picks one. Refreshes run on `handle`.
    pub fn new(env: Env, handle: Handle, limits: Limits) -> Self {
        let shared = Arc::new(Shared { view: watch::channel(View::default()).0, current: Mutex::default() });
        let actors = Actors::new(handle.clone(), limits);
        let messages = Messages::new(handle.clone(), limits);
        let types = Types::new(handle.clone(), limits);
        let watches = Watches::new(handle.clone(), limits);
        Self { env, limits, handle, shared, actors, messages, types, watches }
    }

    /// The Actors page's reads, on the client of the context in use.
    pub fn actors(&self) -> &Actors {
        &self.actors
    }

    /// The Messages page's reads, on the client of the context in use.
    pub fn messages(&self) -> &Messages {
        &self.messages
    }

    /// The Types page on the client of the context in use.
    pub fn types(&self) -> &Types {
        &self.types
    }

    /// The Watches page on the client of the context in use.
    pub fn watches(&self) -> &Watches {
        &self.watches
    }

    /// The views, newest last.
    pub fn view(&self) -> watch::Receiver<View> {
        self.shared.view.subscribe()
    }

    /// Reads the contexts again, makes the client for `picked` (`None`: as `cicada` picks one) and refreshes. The
    /// Overview and the lists and panes of the context before are dropped.
    pub fn use_context(&self, picked: Option<String>) {
        let contexts = Contexts::load(&self.env);
        let target = cicada_app::context_client(&self.env, picked.as_deref());
        let overrides = OVERRIDES.into_iter().filter(|name| self.env.get(*name).is_some_and(|v| !v.is_empty()));
        {
            let mut current = self.shared.current.lock().expect("inspector session");
            current.supersede();
            self.shared.view.send_modify(|view| {
                (view.contexts, view.contexts_error) = match contexts {
                    Ok(contexts) => (contexts, None),
                    Err(error) => (Contexts::default(), Some(error_text(error.as_ref()))),
                };
                view.picked = picked;
                view.overrides = overrides.collect();
                view.overview = None;
                view.refreshing = false;
                match target {
                    Ok((client, context)) => {
                        view.context = context;
                        view.target_error = None;
                        current.client = Some(client);
                    }
                    Err(error) => {
                        view.context = None;
                        view.target_error = Some(error_text(error.as_ref()));
                        current.client = None;
                    }
                }
            });
            self.actors.use_client(current.client.clone());
            self.messages.use_client(current.client.clone());
            self.types.use_client(current.client.clone());
            self.watches.use_client(current.client.clone());
        }
        self.refresh();
    }

    /// Starts a refresh of the context in use, cancelling one that runs. Without a client it does nothing.
    pub fn refresh(&self) {
        let mut current = self.shared.current.lock().expect("inspector session");
        current.supersede();
        let Some(client) = current.client.clone() else { return };
        let generation = current.generation;
        self.shared.view.send_modify(|view| view.refreshing = true);
        let (shared, limits) = (self.shared.clone(), self.limits);
        current.task = Some(self.handle.spawn(async move {
            let overview = overview(&client, limits).await;
            let current = shared.current.lock().expect("inspector session");
            if current.generation == generation {
                shared.view.send_modify(|view| {
                    view.overview = Some(overview);
                    view.refreshing = false;
                });
            }
        }));
    }

    /// The refreshes cancelled so far; each ends once it has stopped, with or without its own end.
    #[cfg(test)]
    pub fn take_superseded(&self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut self.shared.current.lock().expect("inspector session").superseded)
    }

    /// Picks a context as [`Self::use_context`] does and forgets `navigation`'s history, whose places belong to the
    /// context before.
    pub fn switch_context(&self, navigation: &mut Navigation, picked: Option<String>) {
        self.use_context(picked);
        navigation.forget();
    }

    /// Applies `change` to the history, remembering the Messages or Watches place left and restoring the place
    /// reached. Leaving Actors cancels a pending Show Actor, and arriving at a Show Actor place starts it again.
    /// Returns what `change` returned.
    pub fn navigate(&self, navigation: &mut Navigation, change: impl FnOnce(&mut Navigation) -> bool) -> bool {
        let left = navigation.current();
        let recorded = navigation.place().clone();
        match left {
            Section::Messages => navigation.current.messages = Some(self.messages.source()),
            Section::Watches => navigation.current.watches = Some(self.watches.source()),
            _ => {}
        }
        if !change(navigation) {
            navigation.current = recorded;
            return false;
        }
        if left == Section::Actors {
            self.actors.cancel_jump();
        }
        let place = navigation.place();
        if let Some(actor) = &place.actor {
            self.actors.jump(actor.clone());
        }
        if let Some((prefix, selected)) = &place.messages {
            self.messages.restore(prefix, selected.clone());
        }
        if let Some(watch) = &place.watches {
            self.watches.restore(watch.clone());
        }
        true
    }

    /// Show Actor: records a jump to `actor` on the Actors page and starts it.
    pub fn show_actor(&self, navigation: &mut Navigation, actor: ActorAddress) {
        self.navigate(navigation, |navigation| {
            navigation.jump(actor);
            true
        });
    }

    /// Cancels a refresh and the pages' reads, and discards their results; for a window that closes.
    pub fn stop(&self) {
        self.shared.current.lock().expect("inspector session").supersede();
        self.actors.stop();
        self.messages.stop();
        self.types.stop();
        self.watches.stop();
        self.shared.view.send_modify(|view| view.refreshing = false);
    }
}

impl Current {
    fn supersede(&mut self) {
        self.generation += 1;
        if let Some(task) = self.task.take() {
            task.abort();
            #[cfg(test)]
            self.superseded.push(task);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Ok(mut current) = self.shared.current.lock() {
            current.supersede();
        }
    }
}

/// An item of the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    /// The Overview.
    #[default]
    Overview,
    /// Actors (`inspector-actors`).
    Actors,
    /// Pending messages (`inspector-messages`).
    Messages,
    /// Running turns.
    RunningTurns,
    /// Outcomes.
    Outcomes,
    /// Nodes.
    Nodes,
    /// Types.
    Types,
    /// Watches.
    Watches,
}

impl Section {
    /// The items under Browse, in the sidebar's order.
    pub const BROWSE: [Self; 7] =
        [Self::Actors, Self::Messages, Self::RunningTurns, Self::Outcomes, Self::Nodes, Self::Types, Self::Watches];

    /// The sidebar's text.
    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Actors => "Actors",
            Self::Messages => "Messages",
            Self::RunningTurns => "Running Turns",
            Self::Outcomes => "Outcomes",
            Self::Nodes => "Nodes",
            Self::Types => "Types",
            Self::Watches => "Watches",
        }
    }

    /// True for an item with a page; the others say "Coming soon" until their change lands.
    pub fn ready(self) -> bool {
        matches!(self, Self::Overview | Self::Actors | Self::Messages | Self::Types | Self::Watches)
    }
}

/// A place in the history: a sidebar item, with a Show Actor destination or a Messages or Watches selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Place {
    /// The item shown.
    pub section: Section,
    /// The actor a Show Actor selects on the Actors page, again whenever back or forward return here.
    pub actor: Option<ActorAddress>,
    /// The Messages page's prefix and selected message when this place was left, restored on a return to it.
    pub messages: Option<(String, Option<MessageId>)>,
    /// The Watches page's filter and selection when this place was left.
    pub watches: Option<WatchPlace>,
}

/// The sidebar's selection with a back and forward history, as in a browser.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Navigation {
    back: Vec<Place>,
    current: Place,
    forward: Vec<Place>,
}

impl Navigation {
    /// The selected item.
    pub fn current(&self) -> Section {
        self.current.section
    }

    /// The current place.
    pub fn place(&self) -> &Place {
        &self.current
    }

    /// Selects `section`; a new selection drops the forward list. Selecting the current item changes nothing.
    pub fn select(&mut self, section: Section) {
        if section != self.current.section {
            self.go(Place { section, ..Place::default() });
        }
    }

    /// Records a Show Actor: the Actors page with `actor` as its target, after the current place.
    pub fn jump(&mut self, actor: ActorAddress) {
        self.go(Place { section: Section::Actors, actor: Some(actor), ..Place::default() });
    }

    /// Forgets the history and what the current place restores: after a context switch, a place of the context
    /// before would jump to, or restore, a same-named actor or message of another server.
    pub fn forget(&mut self) {
        self.back.clear();
        self.forward.clear();
        self.current = Place { section: self.current.section, ..Place::default() };
    }

    fn go(&mut self, place: Place) {
        self.back.push(std::mem::replace(&mut self.current, place));
        self.forward.clear();
    }

    /// Goes back one selection; false when there is none.
    pub fn back(&mut self) -> bool {
        let Some(place) = self.back.pop() else { return false };
        self.forward.push(std::mem::replace(&mut self.current, place));
        true
    }

    /// Goes forward one selection; false when there is none.
    pub fn forward(&mut self) -> bool {
        let Some(place) = self.forward.pop() else { return false };
        self.back.push(std::mem::replace(&mut self.current, place));
        true
    }

    /// True when [`Self::back`] can go back.
    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// True when [`Self::forward`] can go forward.
    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

/// A tile of the Overview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    /// What it counts.
    pub label: &'static str,
    /// The number or text, `—` when there is none.
    pub value: String,
    /// The small text under the value: what it means, or the error of its call.
    pub note: String,
    /// True when `note` is an error.
    pub error: bool,
}

/// The four tiles of `/stats`: actors, pending, oldest due and failed (kept by retention, which the API does not
/// report). A failed call shows its error on each; a `failed` an older server omits shows `—`.
pub fn tiles(stats: &Part<StatsView>, now: Timestamp) -> [Tile; 4] {
    let tile = |label, value: String, note: &str| Tile { label, value, note: note.to_owned(), error: false };
    match stats {
        Part::Ready(stats) => [
            tile("Actors", stats.actors.to_string(), "in the store"),
            tile("Pending", stats.pending.to_string(), "messages, delayed ones included"),
            match stats.oldest_due {
                Some(due) => tile("Oldest due", age(due, now), "waiting since"),
                None => tile("Oldest due", "—".into(), "nothing due"),
            },
            tile("Failed", stats.failed.map_or_else(|| "—".to_owned(), |n| n.to_string()), "kept by retention"),
        ],
        Part::Failed(error) => ["Actors", "Pending", "Oldest due", "Failed"].map(|label| Tile {
            label,
            value: "—".into(),
            note: error.clone(),
            error: true,
        }),
    }
}

/// How long ago `instant` was, short: `42s`, `5m`, `3h`, `2d`; `in 1m` for one ahead.
pub fn age(instant: Timestamp, now: Timestamp) -> String {
    let seconds = (now - instant).num_seconds();
    let n = seconds.unsigned_abs();
    let short = match n {
        0..60 => format!("{n}s"),
        60..3600 => format!("{}m", n / 60),
        3600..172_800 => format!("{}h", n / 3600),
        _ => format!("{}d", n / 86_400),
    };
    if seconds < 0 { format!("in {short}") } else { short }
}

/// The text of a type's count: the number, `not counted`, or `error`.
pub fn count_text(count: &Count) -> String {
    match count {
        Count::Counted(n) => n.to_string(),
        Count::Failed(_) => "error".into(),
        Count::NotCounted => "not counted".into(),
    }
}
