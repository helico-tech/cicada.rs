//! The actor types of the sample service, each picked by its name with `--types` (`CICADA_TYPES`), and the singletons
//! a node creates for the types it hosts.

use std::sync::Arc;

use cicada::actor::{DefinitionError, contract_of};
use cicada::contract::Contract;
use cicada::{Actor, ActorAddress, ActorRef, ActorType, NoState};
use cicada_actors::{DeadLetters, DeadLettersActor, DeadLettersState, SchedulerActor, SupervisorActor, TopicActor};
use cicada_app::App;
use cicada_examples::cron::CronJobActor;
use cicada_examples::fanout::{BatchActor, BatchItemActor};
use cicada_examples::registry::{DeviceActor, DeviceRegistry, DeviceRegistryActor, RegistryState};
use cicada_examples::reply::{CartActor, Pricing, PricingActor};
use cicada_examples::saga::{InventoryActor, OrderActor, Payments, PaymentsActor};
use cicada_examples::singleton::{Cleanup, CleanupActor};
use cicada_examples::steps::OnboardingActor;
use cicada_examples::sync::{ReportsActor, SyncActor};
use cicada_examples::timer::CheckoutActor;
use serde::Serialize;
use serde_json::Value;

use crate::e2e::{CounterActor, EffectLog, FlakyActor, RecorderActor, SlowActor};
use crate::fakes::Fakes;

/// One actor type of the catalog, with its generic parameters erased.
pub struct CatalogType {
    type_name: &'static str,
    contract: fn() -> Result<Contract, DefinitionError>,
    add: Box<dyn Fn(App) -> App + Send + Sync>,
}

impl CatalogType {
    /// The type of `actor`.
    pub fn of<A: Actor>(actor: A) -> Self {
        let actor = Arc::new(actor);
        Self {
            type_name: <A::Ref as ActorRef>::TYPE,
            contract: contract_of::<A>,
            add: Box::new(move |app| app.host(ActorType::shared(actor.clone()))),
        }
    }

    /// The type's name.
    pub fn type_name(&self) -> &'static str {
        self.type_name
    }

    /// The contract a node hosting this type registers.
    pub fn contract(&self) -> Result<Contract, DefinitionError> {
        (self.contract)()
    }

    /// `app` with this type added.
    pub fn add_to(&self, app: App) -> App {
        (self.add)(app)
    }
}

impl std::fmt::Debug for CatalogType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogType").field("type_name", &self.type_name).finish_non_exhaustive()
    }
}

/// A singleton the node creates at start when it hosts its type (create is idempotent, so every node may).
#[derive(Debug, Clone, PartialEq)]
pub struct Singleton {
    /// Its address.
    pub address: ActorAddress,
    /// Its initial state.
    pub initial_state: Value,
}

impl Singleton {
    fn of<R: ActorRef>(r: &R, initial_state: &impl Serialize) -> Self {
        Self {
            address: r.address().expect("a valid singleton address"),
            initial_state: serde_json::to_value(initial_state).expect("an encodable state"),
        }
    }
}

/// The actor types of the sample service, and their singletons.
#[derive(Debug)]
pub struct Catalog {
    types: Vec<CatalogType>,
    singletons: Vec<Singleton>,
}

impl Catalog {
    /// The catalog of node `node_id`, with the default fakes; the E2E types record their effects in `effects`.
    pub fn new(node_id: &str, effects: EffectLog) -> Self {
        Self::with_fakes(node_id, effects, Fakes::default())
    }

    /// The catalog with the given fakes.
    pub fn with_fakes(node_id: &str, effects: EffectLog, fakes: Fakes) -> Self {
        let types = vec![
            CatalogType::of(CleanupActor { files: fakes.temp_files }),
            CatalogType::of(CheckoutActor),
            CatalogType::of(CronJobActor { task: fakes.cron_task }),
            CatalogType::of(PricingActor { prices: fakes.price_list }),
            CatalogType::of(CartActor),
            CatalogType::of(OrderActor),
            CatalogType::of(InventoryActor),
            CatalogType::of(PaymentsActor { gateway: fakes.payment_gateway }),
            CatalogType::of(OnboardingActor { services: fakes.onboarding }),
            CatalogType::of(BatchActor),
            CatalogType::of(BatchItemActor { processor: fakes.item_processor }),
            CatalogType::of(DeviceRegistryActor),
            CatalogType::of(DeviceActor),
            CatalogType::of(DeadLettersActor::default()),
            CatalogType::of(SupervisorActor),
            CatalogType::of(TopicActor),
            CatalogType::of(SchedulerActor),
            CatalogType::of(SyncActor { api: fakes.sync_api }),
            CatalogType::of(ReportsActor),
            CatalogType::of(SlowActor::new(node_id, effects.clone())),
            CatalogType::of(CounterActor::new(node_id)),
            CatalogType::of(RecorderActor::new(node_id, effects)),
            CatalogType::of(FlakyActor),
        ];
        let singletons = vec![
            Singleton::of(&Cleanup, &NoState {}),
            Singleton::of(&Pricing, &NoState {}),
            Singleton::of(&Payments, &NoState {}),
            Singleton::of(&DeviceRegistry, &RegistryState::default()),
            Singleton::of(&DeadLetters::default(), &DeadLettersState::default()),
        ];
        Self { types, singletons }
    }

    /// Every type, in catalog order.
    pub fn types(&self) -> &[CatalogType] {
        &self.types
    }

    /// Every singleton; a node creates those of the types it hosts.
    pub fn singletons(&self) -> &[Singleton] {
        &self.singletons
    }
}
