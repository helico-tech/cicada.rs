//! A fake engine: containers and images in memory, with switches for failures.

#![allow(dead_code, reason = "each test file uses part of the fake")]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use axum::body::Bytes;
use chrono::{DateTime, Utc};
use cicada::time::Timestamp;
use cicada_docker::{ContainerInfo, ContainerSpec, ContainerStatus, Engine, EngineError, LogOptions};
use futures::StreamExt;
use futures::stream::BoxStream;

#[derive(Debug, Clone)]
pub struct FakeContainer {
    pub id: String,
    pub status: ContainerStatus,
    pub labels: BTreeMap<String, String>,
    pub spec: ContainerSpec,
    pub exit_code: Option<i64>,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
    pub output: Vec<String>,
}

#[derive(Default)]
pub struct State {
    pub containers: BTreeMap<String, FakeContainer>,
    pub images: BTreeSet<String>,
    pub calls: Vec<String>,
    pub down: bool,
    pub fail_once: BTreeMap<&'static str, EngineError>,
    ticks: i64,
}

impl State {
    fn tick(&mut self) -> Timestamp {
        self.ticks += 1;
        DateTime::<Utc>::from_timestamp(1_800_000_000 + self.ticks, 0).unwrap()
    }
}

#[derive(Default, Clone)]
pub struct FakeDocker(Arc<Mutex<State>>);

impl FakeDocker {
    pub fn with_images(images: &[&str]) -> Self {
        let fake = Self::default();
        fake.state().images.extend(images.iter().map(|i| (*i).to_owned()));
        fake
    }

    pub fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap()
    }

    pub fn calls(&self) -> Vec<String> {
        self.state().calls.clone()
    }

    pub fn count(&self, call: &str) -> usize {
        self.state().calls.iter().filter(|c| c.as_str() == call).count()
    }

    pub fn container(&self, name: &str) -> Option<FakeContainer> {
        self.state().containers.get(name).cloned()
    }

    pub fn set_down(&self, down: bool) {
        self.state().down = down;
    }

    pub fn fail_once(&self, op: &'static str, error: EngineError) {
        self.state().fail_once.insert(op, error);
    }

    /// The container exits by itself with `code`, having printed `lines`.
    pub fn exit(&self, name: &str, code: i64, lines: &[&str]) {
        let mut state = self.state();
        let now = state.tick();
        let c = state.containers.get_mut(name).expect("container");
        c.status = ContainerStatus::Exited;
        c.exit_code = Some(code);
        c.finished_at = Some(now);
        c.output.extend(lines.iter().map(|l| (*l).to_owned()));
    }

    pub fn print(&self, name: &str, lines: &[&str]) {
        self.state().containers.get_mut(name).expect("container").output.extend(lines.iter().map(|l| (*l).to_owned()));
    }

    /// A container named `name` that Cicada did not create.
    pub fn put_foreign(&self, name: &str) {
        self.state().containers.insert(
            name.to_owned(),
            FakeContainer {
                id: "foreign".into(),
                status: ContainerStatus::Running,
                labels: BTreeMap::new(),
                spec: ContainerSpec::new("other"),
                exit_code: None,
                started_at: None,
                finished_at: None,
                output: vec!["not yours".into()],
            },
        );
    }

    fn enter(&self, op: &'static str, call: String) -> Result<MutexGuard<'_, State>, EngineError> {
        let mut state = self.state();
        if state.down {
            return Err(EngineError::Other("Docker daemon unreachable".into()));
        }
        if let Some(error) = state.fail_once.remove(op) {
            return Err(error);
        }
        state.calls.push(call);
        Ok(state)
    }
}

#[async_trait]
impl Engine for FakeDocker {
    async fn inspect(&self, name: &str) -> Result<Option<ContainerInfo>, EngineError> {
        let state = self.enter("inspect", format!("inspect {name}"))?;
        Ok(state.containers.get(name).map(|c| ContainerInfo {
            id: c.id.clone(),
            status: c.status,
            labels: c.labels.clone(),
            exit_code: c.exit_code,
            error: None,
            oom_killed: false,
            started_at: c.started_at,
            finished_at: c.finished_at,
        }))
    }

    async fn image_present(&self, image: &str) -> Result<bool, EngineError> {
        let state = self.enter("image_present", format!("image_present {image}"))?;
        Ok(state.images.contains(image))
    }

    async fn pull(&self, image: &str) -> Result<(), EngineError> {
        let mut state = self.enter("pull", format!("pull {image}"))?;
        state.images.insert(image.to_owned());
        Ok(())
    }

    async fn create(
        &self,
        name: &str,
        spec: &ContainerSpec,
        labels: &BTreeMap<String, String>,
    ) -> Result<String, EngineError> {
        let mut state = self.enter("create", format!("create {name}"))?;
        if !state.images.contains(&spec.image) {
            return Err(EngineError::NotFound(format!("No such image: {}", spec.image)));
        }
        if state.containers.contains_key(name) {
            return Err(EngineError::Conflict(format!("The container name \"/{name}\" is already in use")));
        }
        let id = format!("id-{}", state.calls.len());
        let mut all = spec.labels.clone();
        all.extend(labels.clone());
        state.containers.insert(
            name.to_owned(),
            FakeContainer {
                id: id.clone(),
                status: ContainerStatus::Created,
                labels: all,
                spec: spec.clone(),
                exit_code: None,
                started_at: None,
                finished_at: None,
                output: Vec::new(),
            },
        );
        Ok(id)
    }

    async fn start(&self, name: &str) -> Result<(), EngineError> {
        let mut state = self.enter("start", format!("start {name}"))?;
        let now = state.tick();
        let c = state.containers.get_mut(name).ok_or_else(|| EngineError::NotFound(name.to_owned()))?;
        if c.status != ContainerStatus::Running {
            c.status = ContainerStatus::Running;
            c.started_at = Some(now);
            c.exit_code = None;
        }
        Ok(())
    }

    async fn stop(&self, name: &str) -> Result<(), EngineError> {
        let mut state = self.enter("stop", format!("stop {name}"))?;
        let now = state.tick();
        let c = state.containers.get_mut(name).ok_or_else(|| EngineError::NotFound(name.to_owned()))?;
        if c.status == ContainerStatus::Running {
            c.status = ContainerStatus::Exited;
            c.exit_code = Some(0);
            c.finished_at = Some(now);
        }
        Ok(())
    }

    async fn remove(&self, name: &str) -> Result<(), EngineError> {
        let mut state = self.enter("remove", format!("remove {name}"))?;
        state.containers.remove(name);
        Ok(())
    }

    fn logs(&self, name: &str, options: LogOptions) -> BoxStream<'static, Result<Bytes, EngineError>> {
        let lines = match self.enter("logs", format!("logs {name}")) {
            Ok(state) => match state.containers.get(name) {
                Some(c) => c.output.clone(),
                None => return futures::stream::iter([Err(EngineError::NotFound(name.to_owned()))]).boxed(),
            },
            Err(e) => return futures::stream::iter([Err(e)]).boxed(),
        };
        let skip = options.tail.map_or(0, |n| lines.len().saturating_sub(n as usize));
        let chunks: Vec<_> = lines.into_iter().skip(skip).map(|l| Ok(Bytes::from(format!("{l}\n")))).collect();
        futures::stream::iter(chunks).boxed()
    }
}
