//! Example 8 — registry.
//!
//! Two ways to find instances. The registry actor keeps the refs of the devices it created in its state, so a handler
//! can act on all of them (here: broadcast a firmware update) and the set is consistent with its own commits. From
//! outside, `list_actors(prefix)` (`GET /actors?prefix=actor://devices/`) enumerates instances by namespace without
//! any actor.

use std::collections::BTreeMap;

use cicada::prelude::*;
use cicada::time::Timestamp;
use serde::{Deserialize, Serialize};

/// The device registry, a singleton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRegistry;

impl ActorRef for DeviceRegistry {
    const TEMPLATE: &'static str = "actor://registries/devices";
    type Message = RegistryMsg;
}

/// Messages of [`DeviceRegistry`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all_fields = "camelCase")]
pub enum RegistryMsg {
    /// Registers (creates) a device; idempotent.
    RegisterDevice {
        /// The device id.
        device_id: String,
        /// Its model.
        model: String,
    },
    /// Unregisters a device; the device deletes itself.
    UnregisterDevice {
        /// The device id.
        device_id: String,
    },
    /// Sends a firmware update to every registered device.
    BroadcastFirmware {
        /// The firmware version.
        version: String,
    },
}

/// State of the [`DeviceRegistry`]: the refs of all registered devices, by id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RegistryState {
    /// The registered devices.
    pub devices: BTreeMap<String, Device>,
}

/// One device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Device {
    /// The device id.
    pub id: String,
}

impl ActorRef for Device {
    const TEMPLATE: &'static str = "actor://devices/{id}";
    type Message = DeviceMsg;
}

/// Messages of [`Device`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum DeviceMsg {
    /// Installs firmware `version`.
    UpdateFirmware {
        /// The firmware version.
        version: String,
    },
    /// Removes the device.
    Decommission,
}

/// State of a [`Device`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceState {
    /// The model.
    pub model: String,
    /// When the registry created it.
    #[serde(with = "cicada::time::serde_instant")]
    #[schemars(with = "String")]
    pub registered_at: Timestamp,
    /// The installed firmware.
    #[serde(default)]
    pub firmware: Option<String>,
}

/// The registry type.
pub struct DeviceRegistryActor;

impl Actor for DeviceRegistryActor {
    type Ref = DeviceRegistry;
    type State = RegistryState;
    type Message = RegistryMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: RegistryMsg) -> HandlerResult {
        match message {
            RegistryMsg::RegisterDevice { device_id, model } => {
                if turn.state().devices.contains_key(&device_id) {
                    return Ok(());
                }
                let device = Device { id: device_id.clone() };
                let registered_at = turn.now();
                turn.create(&device, &DeviceState { model, registered_at, firmware: None })?;
                turn.state_mut().devices.insert(device_id, device);
            }
            RegistryMsg::UnregisterDevice { device_id } => {
                let Some(device) = turn.state().devices.get(&device_id).cloned() else { return Ok(()) };
                turn.state_mut().devices.remove(&device_id);
                turn.send(&device, DeviceMsg::Decommission)?;
            }
            RegistryMsg::BroadcastFirmware { version } => {
                let devices: Vec<Device> = turn.state().devices.values().cloned().collect();
                for device in devices {
                    turn.send(&device, DeviceMsg::UpdateFirmware { version: version.clone() })?;
                }
            }
        }
        Ok(())
    }
}

/// The device type.
pub struct DeviceActor;

impl Actor for DeviceActor {
    type Ref = Device;
    type State = DeviceState;
    type Message = DeviceMsg;
    const UNHANDLED: Unhandled = Unhandled::Fail;

    async fn handle(&self, turn: &mut Turn<Self>, message: DeviceMsg) -> HandlerResult {
        match message {
            DeviceMsg::UpdateFirmware { version } => turn.state_mut().firmware = Some(version),
            DeviceMsg::Decommission => turn.stop(),
        }
        Ok(())
    }
}
