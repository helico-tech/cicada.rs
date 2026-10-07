//! The actor types of the sample service, in groups selectable with `HOSTED_TYPES`.

use std::sync::Arc;

use cicada::actor::contract_of;
use cicada::contract::Contract;
use cicada::runtime::ActorSystemBuilder;
use cicada::{Actor, ActorAddress, ActorRef, ActorType};
use cicada_examples::cron::CronJobActor;
use cicada_examples::fanout::{BatchActor, BatchItemActor};
use cicada_examples::registry::{DeviceActor, DeviceRegistry, DeviceRegistryActor, RegistryState};
use cicada_examples::reply::{CartActor, Pricing, PricingActor};
use cicada_examples::saga::{InventoryActor, OrderActor, Payments, PaymentsActor};
use cicada_examples::singleton::{Cleanup, CleanupActor};
use cicada_examples::steps::OnboardingActor;
use cicada_examples::sync::{ReportsActor, SyncActor};
use cicada_examples::timer::CheckoutActor;
use cicada_extras::{DeadLetters, DeadLettersActor, DeadLettersState};
use serde::Serialize;
use serde_json::Value;

use crate::e2e::{CounterActor, EffectLog, FlakyActor, RecorderActor, SlowActor};
use crate::fakes::Fakes;

/// All group names, in catalog order.
pub const GROUPS: [&str; 11] =
    ["singleton", "timer", "cron", "reply", "saga", "steps", "fanout", "registry", "supervision", "sync", "e2e"];

/// One actor type of the catalog, with its generic parameters erased.
pub struct CatalogType {
    template: &'static str,
    contract: fn() -> Result<Contract, Vec<String>>,
    host: Box<dyn Fn(ActorSystemBuilder) -> ActorSystemBuilder + Send + Sync>,
}

impl CatalogType {
    /// The type of `actor`.
    pub fn of<A: Actor>(actor: A) -> Self {
        let actor = Arc::new(actor);
        Self {
            template: <A::Ref as ActorRef>::TEMPLATE,
            contract: contract_of::<A>,
            host: Box::new(move |builder| builder.host(ActorType::shared(actor.clone()))),
        }
    }

    /// The type's template.
    pub fn template(&self) -> &'static str {
        self.template
    }

    /// The contract a node hosting this type registers.
    pub fn contract(&self) -> Result<Contract, Vec<String>> {
        (self.contract)()
    }

    /// Hosts this type on the node `builder` builds.
    pub fn host(&self, builder: ActorSystemBuilder) -> ActorSystemBuilder {
        (self.host)(builder)
    }
}

impl std::fmt::Debug for CatalogType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogType").field("template", &self.template).finish_non_exhaustive()
    }
}

/// A singleton the node creates at start when it hosts its group (create is idempotent, so every node may).
#[derive(Debug, Clone, PartialEq)]
pub struct Singleton {
    /// Its address.
    pub address: ActorAddress,
    /// Its initial state; `None` for a stateless type.
    pub initial_state: Option<Value>,
}

impl Singleton {
    fn of<R: ActorRef>(r: &R, initial_state: Option<&impl Serialize>) -> Self {
        Self {
            address: r.address().expect("a valid singleton address"),
            initial_state: initial_state.map(|s| serde_json::to_value(s).expect("an encodable state")),
        }
    }

    fn stateless<R: ActorRef>(r: &R) -> Self {
        Self::of(r, None::<&()>)
    }
}

/// The actor types of the sample service per group, and the singletons per group.
#[derive(Debug)]
pub struct Catalog {
    groups: Vec<(&'static str, Vec<CatalogType>)>,
    singletons: Vec<(&'static str, Vec<Singleton>)>,
}

impl Catalog {
    /// The catalog of node `node_id`, with the default fakes; the E2E types record their effects in `effects`.
    pub fn new(node_id: &str, effects: EffectLog) -> Self {
        Self::with_fakes(node_id, effects, Fakes::default())
    }

    /// The catalog with the given fakes.
    pub fn with_fakes(node_id: &str, effects: EffectLog, fakes: Fakes) -> Self {
        let groups = vec![
            ("singleton", vec![CatalogType::of(CleanupActor { files: fakes.temp_files })]),
            ("timer", vec![CatalogType::of(CheckoutActor)]),
            ("cron", vec![CatalogType::of(CronJobActor { task: fakes.cron_task })]),
            ("reply", vec![CatalogType::of(PricingActor { prices: fakes.price_list }), CatalogType::of(CartActor)]),
            (
                "saga",
                vec![
                    CatalogType::of(OrderActor),
                    CatalogType::of(InventoryActor),
                    CatalogType::of(PaymentsActor { gateway: fakes.payment_gateway }),
                ],
            ),
            ("steps", vec![CatalogType::of(OnboardingActor { services: fakes.onboarding })]),
            (
                "fanout",
                vec![CatalogType::of(BatchActor), CatalogType::of(BatchItemActor { processor: fakes.item_processor })],
            ),
            ("registry", vec![CatalogType::of(DeviceRegistryActor), CatalogType::of(DeviceActor)]),
            ("supervision", vec![CatalogType::of(DeadLettersActor::default())]),
            ("sync", vec![CatalogType::of(SyncActor { api: fakes.sync_api }), CatalogType::of(ReportsActor)]),
            (
                "e2e",
                vec![
                    CatalogType::of(SlowActor::new(node_id, effects.clone())),
                    CatalogType::of(CounterActor::new(node_id)),
                    CatalogType::of(RecorderActor::new(node_id, effects)),
                    CatalogType::of(FlakyActor),
                ],
            ),
        ];
        let singletons = vec![
            ("singleton", vec![Singleton::stateless(&Cleanup)]),
            ("reply", vec![Singleton::stateless(&Pricing)]),
            ("saga", vec![Singleton::stateless(&Payments)]),
            ("registry", vec![Singleton::of(&DeviceRegistry, Some(&RegistryState::default()))]),
            ("supervision", vec![Singleton::of(&DeadLetters, Some(&DeadLettersState::default()))]),
        ];
        Self { groups, singletons }
    }

    /// The types of the `selected` groups, in catalog order.
    pub fn types(&self, selected: &[String]) -> Vec<&CatalogType> {
        self.groups
            .iter()
            .filter(|(name, _)| selected.iter().any(|s| s == name))
            .flat_map(|(_, types)| types)
            .collect()
    }

    /// The singletons of the `selected` groups, in the order of `selected`.
    pub fn singletons(&self, selected: &[String]) -> Vec<&Singleton> {
        selected
            .iter()
            .flat_map(|group| self.singletons.iter().filter(move |(name, _)| name == group))
            .flat_map(|(_, singletons)| singletons)
            .collect()
    }
}

/// Parses `HOSTED_TYPES`: `all`, `none` (a send-only node), or a comma-separated list of [`GROUPS`]. The result keeps
/// the given order, without duplicates.
pub fn parse_groups(value: &str) -> Result<Vec<String>, String> {
    let value = value.trim().to_lowercase();
    match value.as_str() {
        "all" => return Ok(GROUPS.iter().map(|g| (*g).to_owned()).collect()),
        "none" | "" => return Ok(Vec::new()),
        _ => {}
    }
    let mut groups: Vec<String> = Vec::new();
    for group in value.split(',').map(str::trim).filter(|g| !g.is_empty()) {
        if !groups.iter().any(|g| g == group) {
            groups.push(group.to_owned());
        }
    }
    let unknown: Vec<&str> = groups.iter().map(String::as_str).filter(|g| !GROUPS.contains(g)).collect();
    if !unknown.is_empty() {
        return Err(format!(
            "HOSTED_TYPES: unknown group(s) [{}]; known: all, none, {}",
            unknown.join(", "),
            GROUPS.join(", ")
        ));
    }
    Ok(groups)
}
