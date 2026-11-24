//! `DockerRuntime` of `cicada-containers` against the real daemon: the repeat rules, the status and error mappings,
//! `copy_in`, the log pages, the pull, the hardening and the sweep. Everything it makes is named `cicada-rs-e2e-ctr-*` and carries
//! `CONTAINER_LABEL`; the hooks sweep that label, and the suite's teardown removes it after a panic or Ctrl-C.

use std::sync::OnceLock;
use std::time::Duration;

use cicada_containers::{
    ContainerError, ContainerRuntime, ContainerSpec, ContainerStatus, CopyFile, DockerRuntime, ErrorKind, Label,
    LogCursor, LogLine, LogStream, Mount, MountKind, NetworkAttachment, NetworkSpec, Networking, Resource, Tmpfs,
    VolumeSpec,
};
use cicada_e2e::cluster::eventually;
use cicada_e2e::docker::{docker, docker_blocking};
use cicada_e2e::{CONTAINER_LABEL, Class, step};

use crate::secs;

/// An image the stack has pulled already, with GNU tools.
const IMAGE: &str = "postgres:18";
/// The image of the pull, removed first when no container uses it, and of the stop test: it names no stop signal.
const PULLED: &str = "busybox:1.37.0";

pub fn class() -> Class {
    Class {
        before_all: Some(step!(sweep_label).1),
        after_all: Some(step!(sweep_label).1),
        ..Class::new(
            "ContainerDockerTest",
            vec![
                step!(a_repeated_create_keeps_the_first_spec),
                step!(an_exited_container_is_not_run_again),
                step!(stop_ends_with_143_under_init_and_137_without),
                step!(stop_leaves_a_created_container_created),
                step!(what_is_missing_stops_and_removes_ok_and_conflicts_otherwise),
                step!(an_id_prefix_is_not_a_name),
                step!(copy_in_writes_before_start_and_conflicts_after),
                step!(copy_in_on_a_read_only_root_writes_only_to_a_mount),
                step!(log_pages_give_each_line_once_with_its_stream),
                step!(a_follower_collects_every_line_of_a_running_container_once),
                step!(log_pages_of_what_has_no_lines_and_bad_page_sizes),
                step!(a_log_byte_that_is_not_utf8_is_replaced),
                step!(out_of_memory_is_failed),
                step!(a_missing_executable_fails_start_once),
                step!(an_unknown_tag_is_image_not_found),
                step!(a_missing_image_is_pulled_and_a_repeated_create_needs_none),
                step!(a_hardened_container_gets_its_limits_and_secret),
                step!(ensures_repeat_and_an_internal_flag_conflicts),
                step!(a_sweep_removes_in_order_and_skips_a_network_in_use),
            ],
        )
    }
}

/// The backend, on the daemon the rest of the suite reaches through the CLI: `DOCKER_HOST` when it is set, else the
/// endpoint of the CLI's current context (the backend itself reads no context).
fn runtime() -> &'static DockerRuntime {
    static RUNTIME: OnceLock<DockerRuntime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        if std::env::var_os("DOCKER_HOST").is_some() {
            return DockerRuntime::connect().expect("a Docker connection at DOCKER_HOST");
        }
        let context = docker_blocking(&["context", "inspect", "--format", "{{.Endpoints.docker.Host}}"]);
        assert!(context.ok(), "the endpoint of the current Docker context: {}", context.text);
        let host = context.text.trim();
        DockerRuntime::connect_to(host).unwrap_or_else(|e| panic!("a Docker connection at {host}: {e}"))
    })
}

fn suite_label() -> Label {
    Label::new(CONTAINER_LABEL.0, CONTAINER_LABEL.1)
}

/// `cicada-rs-e2e-ctr-<suffix>`.
fn name(suffix: &str) -> String {
    format!("cicada-rs-e2e-ctr-{suffix}")
}

/// A container `name(suffix)` of [`IMAGE`] with the suite's label that runs `script` in `sh -c`.
fn spec(suffix: &str, script: &str) -> ContainerSpec {
    let mut spec = ContainerSpec::new(name(suffix), IMAGE);
    spec.entrypoint = Some(vec!["sh".into(), "-c".into()]);
    spec.command = Some(vec![script.into()]);
    spec.labels.insert(CONTAINER_LABEL.0.into(), CONTAINER_LABEL.1.into());
    spec
}

async fn sweep_label() {
    runtime().sweep(&suite_label()).await.unwrap();
}

async fn status(name: &str) -> ContainerStatus {
    runtime().inspect(name).await.unwrap()
}

/// Waits until the container `name` reports an end state, and returns it.
async fn ended(name: &str) -> ContainerStatus {
    eventually(&format!("{name} ends"), secs(60), async || {
        let status = runtime().inspect(name).await?;
        Ok(matches!(status, ContainerStatus::Exited { .. } | ContainerStatus::Failed { .. }).then_some(status))
    })
    .await
}

/// Waits until the logs of `name` hold `text`.
async fn logs_hold(name: &str, text: &str) -> String {
    eventually(&format!("the logs of {name} hold {text:?}"), secs(60), async || {
        let logs = runtime().logs(name, 100).await?;
        Ok(logs.contains(text).then_some(logs))
    })
    .await
}

fn kind<T: std::fmt::Debug>(result: Result<T, ContainerError>) -> ErrorKind {
    result.unwrap_err().kind
}

/// `docker inspect -f <format> <target>`, trimmed; `None` when docker fails (nothing of that name).
async fn docker_inspect(format: &str, target: &str) -> Option<String> {
    let output = docker(&["inspect", "-f", format, target], secs(60), false).await.unwrap();
    output.ok().then(|| output.text.trim().to_owned())
}

async fn a_repeated_create_keeps_the_first_spec() {
    let first = spec("repeat", "echo first");
    runtime().create(&first).await.unwrap();
    let mut second = spec("repeat", "echo second");
    second.image = PULLED.into();
    runtime().create(&second).await.unwrap();
    assert_eq!(
        docker_inspect("{{.Config.Image}} {{json .Config.Cmd}}", &first.name).await.unwrap(),
        r#"postgres:18 ["echo first"]"#
    );
    assert_eq!(status(&first.name).await, ContainerStatus::Created);
    runtime().remove(&first.name).await.unwrap();
}

async fn an_exited_container_is_not_run_again() {
    let spec = spec("exit-3", "echo ran; exit 3");
    runtime().create(&spec).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    assert_eq!(ended(&spec.name).await, ContainerStatus::Exited { code: 3 });
    runtime().start(&spec.name).await.unwrap();
    tokio::time::sleep(secs(1)).await;
    assert_eq!(status(&spec.name).await, ContainerStatus::Exited { code: 3 });
    assert_eq!(runtime().logs(&spec.name, 100).await.unwrap(), "ran\n", "the program ran once");
    runtime().remove(&spec.name).await.unwrap();
}

async fn stop_ends_with_143_under_init_and_137_without() {
    for (suffix, init, code) in [("stop-init", true, 143), ("stop-pid1", false, 137)] {
        let mut spec = spec(suffix, "");
        // `sleep` as PID 1 ignores SIGTERM; under init it gets it and dies of it. Docker sends the image's stop signal,
        // which is SIGINT for postgres (130), so this runs on busybox, which names none.
        spec.image = PULLED.into();
        spec.entrypoint = Some(vec!["sleep".into(), "600".into()]);
        spec.command = None;
        spec.init = init;
        runtime().create(&spec).await.unwrap();
        runtime().start(&spec.name).await.unwrap();
        assert_eq!(status(&spec.name).await, ContainerStatus::Running);
        runtime().stop(&spec.name, secs(2)).await.unwrap();
        assert_eq!(status(&spec.name).await, ContainerStatus::Exited { code }, "init: {init}");
        runtime().stop(&spec.name, secs(2)).await.unwrap();
        assert_eq!(status(&spec.name).await, ContainerStatus::Exited { code }, "keeps its end");
        runtime().remove(&spec.name).await.unwrap();
    }
}

async fn stop_leaves_a_created_container_created() {
    let spec = spec("stop-created", "echo never");
    runtime().create(&spec).await.unwrap();
    runtime().stop(&spec.name, Duration::ZERO).await.unwrap();
    assert_eq!(status(&spec.name).await, ContainerStatus::Created);
    runtime().remove(&spec.name).await.unwrap();
}

async fn what_is_missing_stops_and_removes_ok_and_conflicts_otherwise() {
    let gone = name("gone");
    assert_eq!(status(&gone).await, ContainerStatus::Missing);
    runtime().stop(&gone, secs(1)).await.unwrap();
    runtime().remove(&gone).await.unwrap();
    assert_eq!(kind(runtime().start(&gone).await), ErrorKind::Conflict);
    assert_eq!(kind(runtime().logs(&gone, 10).await), ErrorKind::Conflict);
    assert_eq!(kind(runtime().copy_in(&gone, &[CopyFile::new("/tmp/a", "x")]).await), ErrorKind::Conflict);

    let spec = spec("remove-twice", "sleep 600");
    runtime().create(&spec).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    runtime().remove(&spec.name).await.unwrap();
    runtime().remove(&spec.name).await.unwrap();
    assert_eq!(status(&spec.name).await, ContainerStatus::Missing);
}

async fn an_id_prefix_is_not_a_name() {
    let spec = spec("prefix", "echo never");
    runtime().create(&spec).await.unwrap();
    let id = docker_inspect("{{.Id}}", &spec.name).await.unwrap();
    let prefix = &id[..12];
    assert_eq!(status(prefix).await, ContainerStatus::Missing);
    runtime().remove(prefix).await.unwrap();
    assert_eq!(status(&spec.name).await, ContainerStatus::Created, "still there");
    runtime().remove(&spec.name).await.unwrap();
}

/// The volume `name(suffix)` with the suite's label.
async fn labelled_volume(suffix: &str) -> String {
    let volume = name(suffix);
    let mut volume_spec = VolumeSpec::new(&volume);
    volume_spec.labels.insert(CONTAINER_LABEL.0.into(), CONTAINER_LABEL.1.into());
    runtime().ensure_volume(&volume_spec).await.unwrap();
    volume
}

/// Whether `path` exists in the running container `name`.
async fn exists(name: &str, path: &str) -> bool {
    docker(&["exec", name, "test", "-e", path], secs(60), false).await.unwrap().ok()
}

async fn copy_in_writes_before_start_and_conflicts_after() {
    let volume = labelled_volume("rw-work").await;
    let mut spec = spec("copy", "stat -c '%a %u' /job/run.sh; /job/run.sh; cat /etc/job.json; sleep 600");
    spec.mounts.push(Mount::new(MountKind::Volume, &volume, "/work"));
    spec.tmpfs.push(Tmpfs::new("/scratch"));
    runtime().create(&spec).await.unwrap();
    // On a writable root too: a file cannot replace a mount point, and the tmpfs mounted at start would hide one.
    let target = [CopyFile::new("/work/x", "x"), CopyFile::new("/work", "x")];
    assert_eq!(kind(runtime().copy_in(&spec.name, &target).await), ErrorKind::InvalidSpec, "the volume target");
    let tmpfs = [CopyFile::new("/etc/x", "x"), CopyFile::new("/scratch/c", "x")];
    assert_eq!(kind(runtime().copy_in(&spec.name, &tmpfs).await), ErrorKind::InvalidSpec, "a path on a tmpfs");
    let mut script = CopyFile::new("/job/run.sh", "#!/bin/sh\necho script ran\n");
    script.mode = 0o755;
    let files = [script, CopyFile::new("/etc/job.json", "{\"job\":1}\n")];
    runtime().copy_in(&spec.name, &files).await.unwrap();
    runtime().copy_in(&spec.name, &files).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    let logs = logs_hold(&spec.name, "{\"job\":1}").await;
    assert_eq!(logs, "755 0\nscript ran\n{\"job\":1}\n");

    for refused in ["/work/x", "/etc/x"] {
        assert!(!exists(&spec.name, refused).await, "{refused} not written");
    }

    let late = [CopyFile::new("/job/late", "x")];
    assert_eq!(kind(runtime().copy_in(&spec.name, &late).await), ErrorKind::Conflict);
    assert!(!exists(&spec.name, "/job/late").await, "no file written");
    runtime().remove(&spec.name).await.unwrap();
}

async fn copy_in_on_a_read_only_root_writes_only_to_a_mount() {
    let volume = labelled_volume("ro-work").await;
    let mut spec = spec("ro-copy", "cat /work/a; test -e /work/x && echo x written; ls /etc/b 2>&1 || true");
    spec.read_only_root = true;
    spec.mounts.push(Mount::new(MountKind::Volume, &volume, "/work"));
    spec.tmpfs.push(Tmpfs::new("/scratch"));
    runtime().create(&spec).await.unwrap();

    let both = [CopyFile::new("/work/a", "on the volume\n"), CopyFile::new("/etc/b", "on the root\n")];
    assert_eq!(kind(runtime().copy_in(&spec.name, &both).await), ErrorKind::InvalidSpec);
    let tmpfs = [CopyFile::new("/scratch/c", "x")];
    assert_eq!(kind(runtime().copy_in(&spec.name, &tmpfs).await), ErrorKind::InvalidSpec, "a tmpfs is no mount yet");
    let target = [CopyFile::new("/work/x", "x"), CopyFile::new("/work/", "x")];
    assert_eq!(kind(runtime().copy_in(&spec.name, &target).await), ErrorKind::InvalidSpec, "the volume target");
    runtime().copy_in(&spec.name, &both[..1]).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    assert_eq!(ended(&spec.name).await, ContainerStatus::Exited { code: 0 });
    let logs = runtime().logs(&spec.name, 100).await.unwrap();
    assert!(logs.starts_with("on the volume\n"), "{logs}");
    assert!(logs.contains("No such file"), "nothing written to the root: {logs}");
    assert!(!logs.contains("x written"), "nothing written with the mount target: {logs}");
    runtime().remove(&spec.name).await.unwrap();
}

/// Every line of `name` from the start, in pages of `max_lines`, until a page has fewer.
async fn read_to_end(name: &str, max_lines: u32) -> Vec<LogLine> {
    let mut lines = Vec::new();
    let mut next = None;
    loop {
        let page = runtime().logs_since(name, next, max_lines).await.unwrap();
        let short = page.lines.len() < max_lines as usize;
        lines.extend(page.lines);
        next = Some(page.next);
        if short {
            return lines;
        }
    }
}

fn texts(lines: &[LogLine], stream: LogStream) -> Vec<&str> {
    lines.iter().filter(|line| line.stream == stream).map(|line| line.text.as_str()).collect()
}

async fn log_pages_give_each_line_once_with_its_stream() {
    let script =
        r#"for i in $(seq 1 50); do if [ $((i % 2)) = 0 ]; then echo "out $i"; else echo "err $i" >&2; fi; done"#;
    let spec = spec("log-pages", script);
    runtime().create(&spec).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    assert_eq!(ended(&spec.name).await, ContainerStatus::Exited { code: 0 });

    let paged = read_to_end(&spec.name, 7).await;
    assert_eq!(paged.len(), 50, "{paged:?}");
    let outs: Vec<String> = (1..=50).filter(|i| i % 2 == 0).map(|i| format!("out {i}")).collect();
    let errs: Vec<String> = (1..=50).filter(|i| i % 2 == 1).map(|i| format!("err {i}")).collect();
    assert_eq!(texts(&paged, LogStream::Out), outs);
    assert_eq!(texts(&paged, LogStream::Err), errs);
    // Pages of 7 give the lines of one whole read, in its log order.
    let whole = runtime().logs_since(&spec.name, None, 1000).await.unwrap();
    assert_eq!(paged, whole.lines);

    // The same cursor twice gives the same page, also after a trip through JSON.
    let first = runtime().logs_since(&spec.name, None, 7).await.unwrap();
    let again = runtime().logs_since(&spec.name, Some(first.next), 7).await.unwrap();
    let stored: LogCursor = serde_json::from_str(&serde_json::to_string(&first.next).unwrap()).unwrap();
    assert_eq!(runtime().logs_since(&spec.name, Some(stored), 7).await.unwrap(), again);
    assert_eq!(again.lines, whole.lines[7..14]);
    runtime().remove(&spec.name).await.unwrap();
}

async fn a_follower_collects_every_line_of_a_running_container_once() {
    let spec = spec("log-follow", r#"for i in $(seq 1 10); do echo "line $i"; sleep 0.3; done"#);
    runtime().create(&spec).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    let mut collected = Vec::new();
    let mut next = None;
    let mut exited = false;
    loop {
        let page = runtime().logs_since(&spec.name, next, 2).await.unwrap();
        let short = page.lines.len() < 2;
        if page.lines.is_empty() {
            assert_eq!(Some(page.next), next.or(Some("0:0".parse().unwrap())), "an empty page keeps the cursor");
        }
        collected.extend(page.lines.into_iter().map(|line| line.text));
        next = Some(page.next);
        if short && exited {
            break;
        }
        // Once ended, read on until a short page; until then, poll gently.
        exited = matches!(status(&spec.name).await, ContainerStatus::Exited { .. });
        if !exited {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    let expected: Vec<String> = (1..=10).map(|i| format!("line {i}")).collect();
    assert_eq!(collected, expected);
    runtime().remove(&spec.name).await.unwrap();
}

async fn log_pages_of_what_has_no_lines_and_bad_page_sizes() {
    let missing = name("log-missing");
    assert_eq!(kind(runtime().logs_since(&missing, None, 5).await), ErrorKind::Conflict);
    let spec = spec("log-created", "echo never");
    runtime().create(&spec).await.unwrap();
    let page = runtime().logs_since(&spec.name, None, 5).await.unwrap();
    assert!(page.lines.is_empty(), "{page:?}");
    assert_eq!(page.next, "0:0".parse().unwrap());
    for bad in [0, 1001] {
        assert_eq!(kind(runtime().logs_since(&spec.name, None, bad).await), ErrorKind::InvalidSpec, "{bad}");
        assert_eq!(kind(runtime().logs_since(&missing, None, bad).await), ErrorKind::InvalidSpec, "{bad}");
    }
    runtime().remove(&spec.name).await.unwrap();
}

async fn a_log_byte_that_is_not_utf8_is_replaced() {
    let spec = spec("log-utf8", r"printf '\377\n'");
    runtime().create(&spec).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    assert_eq!(ended(&spec.name).await, ContainerStatus::Exited { code: 0 });
    let page = runtime().logs_since(&spec.name, None, 10).await.unwrap();
    let lines: Vec<(LogStream, &str)> = page.lines.iter().map(|line| (line.stream, line.text.as_str())).collect();
    assert_eq!(lines, [(LogStream::Out, "\u{fffd}")]);
    runtime().remove(&spec.name).await.unwrap();
}

async fn out_of_memory_is_failed() {
    let mut spec = spec("oom", "tail /dev/zero");
    spec.memory_bytes = Some(32 << 20);
    runtime().create(&spec).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    match ended(&spec.name).await {
        ContainerStatus::Failed { reason, .. } => assert_eq!(reason, "out of memory"),
        other => panic!("not failed: {other:?}"),
    }
    runtime().remove(&spec.name).await.unwrap();
}

async fn a_missing_executable_fails_start_once() {
    let mut spec = spec("no-exec", "");
    spec.entrypoint = Some(vec!["/no/such/executable".into()]);
    spec.command = None;
    runtime().create(&spec).await.unwrap();
    let error = runtime().start(&spec.name).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidSpec, "{error}");
    assert!(matches!(status(&spec.name).await, ContainerStatus::Failed { .. }), "{:?}", status(&spec.name).await);
    runtime().start(&spec.name).await.unwrap();
    runtime().remove(&spec.name).await.unwrap();
}

async fn an_unknown_tag_is_image_not_found() {
    let mut spec = spec("unknown-tag", "true");
    spec.image = "busybox:cicada-rs-no-such-tag".into();
    let error = runtime().create(&spec).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::ImageNotFound, "{error}");
    assert_eq!(status(&spec.name).await, ContainerStatus::Missing);
}

async fn a_missing_image_is_pulled_and_a_repeated_create_needs_none() {
    let users = docker(&["ps", "-aq", "--filter", &format!("ancestor={PULLED}")], secs(60), true).await.unwrap();
    if users.text.trim().is_empty() {
        docker(&["image", "rm", PULLED], secs(60), false).await.unwrap();
    }
    let mut spec = spec("pulled", "echo pulled");
    spec.image = PULLED.into();
    runtime().create(&spec).await.unwrap();
    assert!(docker_inspect("{{.Id}}", PULLED).await.is_some(), "pulled");

    // The image goes; the container stays, and a repeated create keeps it without pulling.
    docker(&["image", "rm", "-f", PULLED], secs(60), true).await.unwrap();
    runtime().create(&spec).await.unwrap();
    assert!(docker_inspect("{{.Id}}", PULLED).await.is_none(), "no pull");
    assert_eq!(status(&spec.name).await, ContainerStatus::Created);
    runtime().remove(&spec.name).await.unwrap();
}

async fn a_hardened_container_gets_its_limits_and_secret() {
    let script = "touch /rootfile 2>/dev/null && echo root writable || echo root read-only; \
                  touch /scratch/f && echo tmpfs writable; \
                  grep -E '^(CapEff|NoNewPrivs)' /proc/self/status; echo \"token=$TOKEN plain=$PLAIN\"";
    let mut spec = spec("hardened", script);
    spec.read_only_root = true;
    spec.drop_all_capabilities = true;
    spec.no_new_privileges = true;
    spec.init = true;
    spec.pids_limit = Some(64);
    spec.cpu_millis = Some(500);
    spec.memory_bytes = Some(64 << 20);
    spec.tmpfs.push(Tmpfs::new("/scratch"));
    spec.env.insert("PLAIN".into(), "visible".into());
    spec.secret_env.insert("TOKEN".into(), "s3cret-e2e".into());
    runtime().create(&spec).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    assert_eq!(ended(&spec.name).await, ContainerStatus::Exited { code: 0 });
    let logs = runtime().logs(&spec.name, 100).await.unwrap();
    assert!(logs.contains("root read-only\n"), "{logs}");
    assert!(logs.contains("tmpfs writable\n"), "{logs}");
    assert!(logs.contains("CapEff:\t0000000000000000\n"), "{logs}");
    assert!(logs.contains("NoNewPrivs:\t1\n"), "{logs}");
    assert!(logs.contains("token=s3cret-e2e plain=visible\n"), "{logs}");
    let limits = "{{.HostConfig.PidsLimit}} {{.HostConfig.NanoCpus}} {{.HostConfig.Memory}} {{.HostConfig.MemorySwap}}";
    let expected = format!("64 500000000 {} {}", 64 << 20, 64 << 20);
    assert_eq!(docker_inspect(limits, &spec.name).await.unwrap(), expected);
    runtime().remove(&spec.name).await.unwrap();
}

async fn ensures_repeat_and_an_internal_flag_conflicts() {
    let mut network = NetworkSpec::new(name("net-internal"));
    network.internal = true;
    network.labels.insert(CONTAINER_LABEL.0.into(), CONTAINER_LABEL.1.into());
    runtime().ensure_network(&network).await.unwrap();
    runtime().ensure_network(&network).await.unwrap();
    let mut open = network.clone();
    open.internal = false;
    assert_eq!(kind(runtime().ensure_network(&open).await), ErrorKind::Conflict);
    assert_eq!(docker_inspect("{{.Internal}}", &network.name).await.unwrap(), "true");

    let mut volume = VolumeSpec::new(name("vol-repeat"));
    volume.labels.insert(CONTAINER_LABEL.0.into(), CONTAINER_LABEL.1.into());
    runtime().ensure_volume(&volume).await.unwrap();
    runtime().ensure_volume(&volume).await.unwrap();
    let listed = docker(&["volume", "ls", "-q", "--filter", &format!("name={}", volume.name)], secs(60), true).await;
    assert_eq!(listed.unwrap().text.trim(), volume.name);
}

async fn a_sweep_removes_in_order_and_skips_a_network_in_use() {
    const RUN: &str = "cicada-rs-e2e-run";
    for run in ["a", "b"] {
        let mut network = NetworkSpec::new(name(&format!("sweep-net-{run}")));
        let mut volume = VolumeSpec::new(name(&format!("sweep-vol-{run}")));
        for labels in [&mut network.labels, &mut volume.labels] {
            labels.insert(CONTAINER_LABEL.0.into(), CONTAINER_LABEL.1.into());
            labels.insert(RUN.into(), run.into());
        }
        runtime().ensure_network(&network).await.unwrap();
        runtime().ensure_volume(&volume).await.unwrap();
        for n in 1..=2 {
            let mut spec = spec(&format!("sweep-{run}{n}"), "sleep 600");
            spec.labels.insert(RUN.into(), run.into());
            spec.network = Networking::Attach(vec![NetworkAttachment::new(&network.name)]);
            spec.mounts.push(Mount::new(MountKind::Volume, &volume.name, "/work"));
            runtime().create(&spec).await.unwrap();
            runtime().start(&spec.name).await.unwrap();
        }
    }
    // A running container without the run label on the network of `a`.
    let mut user = spec("sweep-user", "sleep 600");
    user.network = Networking::Attach(vec![NetworkAttachment::new(name("sweep-net-a"))]);
    runtime().create(&user).await.unwrap();
    runtime().start(&user.name).await.unwrap();

    let swept = runtime().sweep(&Label::new(RUN, "a")).await.unwrap();
    assert_eq!(swept.containers, [name("sweep-a1"), name("sweep-a2")]);
    assert!(swept.networks.is_empty(), "{swept:?}");
    assert_eq!(swept.volumes, [name("sweep-vol-a")]);
    assert_eq!(swept.skipped.len(), 1, "{swept:?}");
    assert_eq!(
        (swept.skipped[0].resource, swept.skipped[0].name.as_str()),
        (Resource::Network, name("sweep-net-a").as_str())
    );
    assert!(!swept.skipped[0].reason.is_empty());

    let b = runtime().list(&Label::new(RUN, "b")).await.unwrap();
    let names: Vec<&str> = b.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, [name("sweep-b1"), name("sweep-b2")]);
    assert!(b.iter().all(|s| s.status == ContainerStatus::Running));
    assert!(docker_inspect("{{.Name}}", &name("sweep-net-b")).await.is_some());
    assert!(docker_inspect("{{.Name}}", &name("sweep-vol-b")).await.is_some());
    assert!(docker_inspect("{{.Name}}", &name("sweep-net-a")).await.is_some(), "skipped, still there");
    assert_eq!(status(&name("sweep-a1")).await, ContainerStatus::Missing);

    runtime().remove(&user.name).await.unwrap();
    let rest = runtime().sweep(&suite_label()).await.unwrap();
    assert!(rest.skipped.is_empty(), "{rest:?}");
    assert!(rest.networks.contains(&name("sweep-net-a")), "{rest:?}");
}
