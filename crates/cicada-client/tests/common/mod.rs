//! Actor types and a real HTTP server for the client tests.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::Router;
use cicada::Store;
use cicada::prelude::*;
use cicada::runtime::{ActorSystemBuilder, NodeConfig};
use cicada_axum::actor_api;
use cicada_client::ActorClient;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

/// An account of a tenant: its name is `tenant:id`, the address the ref with these two fields had before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
}

impl Account {
    pub fn tenant(&self) -> &str {
        self.name.rsplit_once(':').map_or(&self.name, |(tenant, _)| tenant)
    }
}

impl ActorRef for Account {
    const TYPE: &'static str = "account";
    type Message = AccountMsg;
}

pub fn account(tenant: &str, id: i32) -> Account {
    Account { name: format!("{tenant}:{id}") }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum AccountMsg {
    Deposit {
        amount: i64,
    },
    Hold {
        gate: String,
    },
    Explode,
    /// Deposits `amount` into the account `to` of the same tenant.
    Transfer {
        to: i32,
        amount: i64,
    },
    /// Creates the account `id` of the same tenant as a child of this one.
    Open {
        id: i32,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccountState {
    #[serde(default)]
    pub balance: i64,
    #[serde(default)]
    pub owner: String,
}

pub fn owned_by(owner: &str) -> AccountState {
    AccountState { balance: 0, owner: owner.to_owned() }
}

pub struct AccountActor;

impl Actor for AccountActor {
    type Ref = Account;
    type State = AccountState;

    async fn handle(&self, turn: &mut Turn<Self>, message: AccountMsg) -> HandlerResult {
        match message {
            AccountMsg::Deposit { amount } => turn.state_mut().balance += amount,
            AccountMsg::Hold { gate } => gates::wait(&gate).await,
            AccountMsg::Explode => return Err(HandlerError::msg("boom")),
            AccountMsg::Transfer { to, amount } => {
                let to = account(turn.self_ref().tenant(), to);
                turn.send(&to, AccountMsg::Deposit { amount })?;
            }
            AccountMsg::Open { id } => {
                let child = account(turn.self_ref().tenant(), id);
                turn.create_child(&child, &AccountState::default())?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Beacon {
    pub name: String,
}

impl ActorRef for Beacon {
    const TYPE: &'static str = "beacon";
    type Message = BeaconMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum BeaconMsg {
    Blink,
}

pub struct BeaconActor;

impl Actor for BeaconActor {
    type Ref = Beacon;
    type State = NoState;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: BeaconMsg) -> HandlerResult {
        Ok(())
    }
}

/// type `watcher`: takes the notifications of watches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watcher {
    pub id: String,
}

impl ActorRef for Watcher {
    const TYPE: &'static str = "watcher";
    type Message = WatcherMsg;
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum WatcherMsg {
    Failed(cicada::Failed),
    Stopped(cicada::Stopped),
}

pub struct WatcherActor;

impl Actor for WatcherActor {
    type Ref = Watcher;
    type State = NoState;

    async fn handle(&self, _turn: &mut Turn<Self>, _message: WatcherMsg) -> HandlerResult {
        Ok(())
    }
}

/// Named latches that `Hold` handlers wait on.
pub mod gates {
    use super::*;

    fn gate(name: &str) -> Arc<watch::Sender<bool>> {
        static GATES: OnceLock<Mutex<HashMap<String, Arc<watch::Sender<bool>>>>> = OnceLock::new();
        let mut gates = GATES.get_or_init(Default::default).lock().unwrap();
        gates.entry(name.to_owned()).or_insert_with(|| Arc::new(watch::channel(false).0)).clone()
    }

    pub async fn wait(name: &str) {
        let mut open = gate(name).subscribe();
        let _ = open.wait_for(|o| *o).await;
    }

    pub fn open(name: &str) {
        gate(name).send_replace(true);
    }
}

/// A started node on an in-memory store (real time), hosting the account and beacon types.
pub async fn node() -> ActorSystem {
    node_with(|b| b.host(AccountActor).host(BeaconActor)).await
}

pub async fn node_with(configure: impl FnOnce(ActorSystemBuilder) -> ActorSystemBuilder) -> ActorSystem {
    let store = Store::memory();
    let config = NodeConfig { poll_interval: Duration::from_millis(50), ..NodeConfig::default() };
    let system = configure(ActorSystem::builder(store).config(config)).build();
    system.start().await.expect("the node starts");
    system
}

/// Serves `app` on a random local port; returns its base URL.
pub async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.expect("the server runs") });
    format!("http://{address}")
}

/// A node serving the API, `wrap`ped by the host (middleware), and a client for it.
pub async fn client_test(wrap: impl FnOnce(Router) -> Router) -> (ActorClient, String, ActorSystem) {
    let system = node().await;
    let base = serve(wrap(actor_api(system.clone()))).await;
    let client = ActorClient::new(&base).retry_delay(Duration::from_millis(10));
    (client, base, system)
}

/// Polls `condition` every 20 ms until it holds, for at most 10 s.
pub async fn eventually(mut condition: impl AsyncFnMut() -> bool) {
    for _ in 0..500 {
        if condition().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("Condition not met within 10 s");
}
