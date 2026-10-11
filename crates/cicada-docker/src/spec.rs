use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What to run: the image and how to run it. A subset of Docker's container settings; anything else is Docker's
/// default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerSpec {
    /// Image reference, for example `nginx:1.27` or `ghcr.io/acme/job@sha256:…`.
    pub image: String,
    /// Replaces the image's `CMD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<Vec<String>>,
    /// Replaces the image's `ENTRYPOINT`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<Vec<String>>,
    /// Environment variables.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Extra labels; the `cicada.` prefix is reserved.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    /// Published ports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<PortBinding>,
    /// Bind mounts and volumes in Docker's `source:target[:options]` form.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub binds: Vec<String>,
    /// Network mode or network name; Docker's default bridge when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    /// Docker's restart policy. The handle never restarts a container itself (ADR 0007).
    #[serde(default)]
    pub restart: RestartPolicy,
    /// When to pull the image.
    #[serde(default)]
    pub pull: PullPolicy,
    /// Seconds Docker waits after `SIGTERM` before it kills the container on stop; Docker's default (10) when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_timeout_seconds: Option<u32>,
}

impl ContainerSpec {
    /// A spec that runs `image` with its defaults.
    pub fn new(image: impl Into<String>) -> Self {
        Self { image: image.into(), ..Self::default() }
    }

    /// Replaces the image's `CMD` with `cmd`.
    pub fn cmd<S: Into<String>>(mut self, cmd: impl IntoIterator<Item = S>) -> Self {
        self.cmd = Some(cmd.into_iter().map(Into::into).collect());
        self
    }

    /// Sets the environment variable `name`.
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(name.into(), value.into());
        self
    }

    /// Publishes `container_port` (TCP) on `host_port`.
    pub fn port(mut self, container_port: u16, host_port: u16) -> Self {
        self.ports.push(PortBinding { container_port, host_port: Some(host_port), protocol: Protocol::Tcp });
        self
    }

    /// Sets the restart policy.
    pub fn restart(mut self, restart: RestartPolicy) -> Self {
        self.restart = restart;
        self
    }

    /// Sets the pull policy.
    pub fn pull(mut self, pull: PullPolicy) -> Self {
        self.pull = pull;
        self
    }

    /// A stable fingerprint of the spec: the hex SHA-256 of its JSON. Stored on the container as `cicada.spec-hash`;
    /// a container whose hash differs from the applied spec is replaced.
    pub fn hash(&self) -> String {
        let json = serde_json::to_vec(self).expect("a container spec always serializes");
        Sha256::digest(&json).iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// One published port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortBinding {
    /// Port inside the container.
    pub container_port: u16,
    /// Port on the host; Docker picks a free one when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_port: Option<u16>,
    /// Protocol.
    #[serde(default)]
    pub protocol: Protocol,
}

/// Port protocol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// TCP.
    #[default]
    Tcp,
    /// UDP.
    Udp,
}

impl Protocol {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

/// Docker's restart policy for the container.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum RestartPolicy {
    /// Never restart (a container that exits stays exited).
    #[default]
    No,
    /// Restart when it exits with a non-zero code, at most `max_retries` times (unbounded when absent).
    OnFailure {
        /// Maximum number of restarts.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_retries: Option<u32>,
    },
    /// Restart whenever it exits, unless it was stopped.
    UnlessStopped,
    /// Restart whenever it exits.
    Always,
}

/// When to pull the image before creating the container.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PullPolicy {
    /// Pull only when the daemon does not have the image.
    #[default]
    Missing,
    /// Pull on every create.
    Always,
    /// Never pull; creating fails when the image is absent.
    Never,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_sees_changes() {
        let spec = ContainerSpec::new("alpine:3.20").cmd(["sleep", "60"]).env("A", "1");
        assert_eq!(spec.hash(), spec.clone().hash());
        assert_eq!(spec.hash().len(), 64);
        assert_ne!(spec.hash(), spec.clone().env("A", "2").hash());
    }

    #[test]
    fn wire_form() {
        let spec =
            ContainerSpec::new("alpine:3.20").port(80, 8080).restart(RestartPolicy::OnFailure { max_retries: Some(3) });
        assert_eq!(
            serde_json::to_value(&spec).unwrap(),
            serde_json::json!({
                "image": "alpine:3.20",
                "ports": [{ "containerPort": 80, "hostPort": 8080, "protocol": "tcp" }],
                "restart": { "type": "on-failure", "maxRetries": 3 },
                "pull": "missing"
            })
        );
        let decoded: ContainerSpec = serde_json::from_value(serde_json::json!({ "image": "alpine" })).unwrap();
        assert_eq!(decoded, ContainerSpec::new("alpine"));
    }
}
