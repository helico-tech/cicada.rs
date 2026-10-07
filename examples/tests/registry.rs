//! Example 8: the device registry.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use cicada_examples::registry::{Device, DeviceActor, DeviceRegistry, DeviceRegistryActor, DeviceState, RegistryMsg, RegistryState};
use cicada_test::TestActors;

async fn harness() -> TestActors {
    TestActors::builder().with(|node| node.host(DeviceRegistryActor).host(DeviceActor)).build().await.unwrap()
}

fn device(id: &str) -> Device {
    Device { id: id.into() }
}

async fn register_three(test: &TestActors) {
    test.create(&DeviceRegistry, &RegistryState::default()).await;
    for id in ["d1", "d2", "d3"] {
        test.send(&DeviceRegistry, RegistryMsg::RegisterDevice { device_id: id.into(), model: "thermo".into() }).await;
    }
}

#[tokio::test(start_paused = true)]
async fn the_registry_keeps_the_refs_of_the_devices_it_created() {
    let test = harness().await;
    register_three(&test).await;
    // Already registered.
    test.send(&DeviceRegistry, RegistryMsg::RegisterDevice { device_id: "d1".into(), model: "other".into() }).await;
    test.run_until_idle().await;

    let devices = test.state::<_, RegistryState>(&DeviceRegistry).await.devices;
    let expected: BTreeMap<_, _> = ["d1", "d2", "d3"].map(|id| (id.to_owned(), device(id))).into();
    assert_eq!(devices, expected);
    assert_eq!(test.state::<_, DeviceState>(&device("d1")).await.model, "thermo");
    // The other way to find them: list by namespace prefix, no actor needed.
    assert_eq!(
        common::list(&test, "actor://devices/").await,
        vec!["actor://devices/d1", "actor://devices/d2", "actor://devices/d3"]
    );
}

#[tokio::test(start_paused = true)]
async fn unregistering_removes_the_device_and_a_broadcast_reaches_the_remaining_ones() {
    let test = harness().await;
    register_three(&test).await;
    test.send(&DeviceRegistry, RegistryMsg::UnregisterDevice { device_id: "d2".into() }).await;
    test.send(&DeviceRegistry, RegistryMsg::BroadcastFirmware { version: "2.1.0".into() }).await;
    test.run_until_idle().await;

    assert!(!test.exists(&device("d2")).await);
    let devices = test.state::<_, RegistryState>(&DeviceRegistry).await.devices;
    assert_eq!(devices.keys().map(String::as_str).collect::<BTreeSet<_>>(), BTreeSet::from(["d1", "d3"]));
    assert_eq!(test.state::<_, DeviceState>(&device("d1")).await.firmware.as_deref(), Some("2.1.0"));
    assert_eq!(test.state::<_, DeviceState>(&device("d3")).await.firmware.as_deref(), Some("2.1.0"));
    assert_eq!(common::list(&test, "actor://devices/").await, vec!["actor://devices/d1", "actor://devices/d3"]);
}
