//! `DockerRuntime` of `cicada-containers` against the real daemon: the runtime info, the repeat rules, the status and error mappings,
//! `copy_in`, `make_dirs`, `copy_out`, the log pages, the pull, the pull from a private registry, the hardening and the sweep. Everything it makes is named `cicada-rs-e2e-ctr-*` and carries
//! `CONTAINER_LABEL`; the hooks sweep that label, and the suite's teardown removes it after a panic or Ctrl-C.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use cicada_containers::{
    ContainerError, ContainerRuntime, ContainerSpec, ContainerStatus, CopyDir, CopyFile, CopyLimits, CopyOut,
    DockerRuntime, ErrorKind, Label, LogCursor, LogLine, LogStream, Mount, MountKind, NetworkAttachment, NetworkSpec,
    Networking, Resource, RuntimeInfo, Tmpfs, VolumeSpec,
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
                step!(info_gives_linux_and_the_arch_the_cli_prints),
                step!(a_repeated_create_keeps_the_first_spec),
                step!(an_exited_container_is_not_run_again),
                step!(stop_ends_with_143_under_init_and_137_without),
                step!(stop_leaves_a_created_container_created),
                step!(what_is_missing_stops_and_removes_ok_and_conflicts_otherwise),
                step!(an_id_prefix_is_not_a_name),
                step!(copy_in_writes_before_start_and_conflicts_after),
                step!(copy_in_on_a_read_only_root_writes_only_to_a_mount),
                step!(make_dirs_gives_user_1000_a_folder_and_leaves_existing_ones_alone),
                step!(make_dirs_refuses_a_file_and_a_path_below_a_file_and_a_late_call),
                step!(make_dirs_on_a_read_only_root_makes_only_on_a_mount),
                step!(copy_out_collects_regular_files_and_skips_links),
                step!(copy_out_keeps_within_its_limits),
                step!(copy_out_after_the_end_reads_a_volume_and_not_a_tmpfs),
                step!(copy_out_reads_a_progress_file_whole_while_it_is_renamed),
                step!(copy_out_reads_copy_in_back_and_refuses_what_is_not_there),
                step!(log_pages_give_each_line_once_with_its_stream),
                step!(a_follower_collects_every_line_of_a_running_container_once),
                step!(log_pages_of_what_has_no_lines_and_bad_page_sizes),
                step!(a_log_byte_that_is_not_utf8_is_replaced),
                step!(out_of_memory_is_failed),
                step!(a_missing_executable_fails_start_once),
                step!(an_unknown_tag_is_image_not_found),
                step!(a_missing_image_is_pulled_and_a_repeated_create_needs_none),
                step!(a_public_image_is_pulled_past_a_helper_that_is_not_installed),
                step!(a_private_image_is_pulled_with_the_login_of_config_json),
                step!(a_hardened_container_gets_its_limits_and_secret),
                step!(ensures_repeat_and_an_internal_flag_conflicts),
                step!(a_sweep_removes_in_order_and_skips_a_network_in_use),
            ],
        )
    }
}

/// The backend, on the daemon the rest of the suite reaches through the CLI: see [`connect`].
fn runtime() -> &'static DockerRuntime {
    static RUNTIME: OnceLock<DockerRuntime> = OnceLock::new();
    RUNTIME.get_or_init(connect)
}

/// A new backend from `DockerRuntime::connect()` alone, which finds the daemon through `DOCKER_HOST`, `DOCKER_CONTEXT`,
/// the local socket and the CLI's current context, so the class passes on a machine without `/var/run/docker.sock`.
/// On a machine with both a working local socket and another current context, the backend uses the socket while the
/// CLI uses the context: the suite then needs `DOCKER_CONTEXT` or `DOCKER_HOST`, which both read.
fn connect() -> DockerRuntime {
    DockerRuntime::connect().unwrap_or_else(|e| panic!("a Docker connection: {e}"))
}

/// The endpoint of the CLI's current context, for the CLI of the registry test, whose own config folder holds no
/// context; `None` when `DOCKER_HOST` is set, which the CLI reads.
fn context_host() -> Option<&'static str> {
    static HOST: OnceLock<Option<String>> = OnceLock::new();
    HOST.get_or_init(|| {
        if std::env::var_os("DOCKER_HOST").is_some() {
            return None;
        }
        let context = docker_blocking(&["context", "inspect", "--format", "{{.Endpoints.docker.Host}}"]);
        assert!(context.ok(), "the endpoint of the current Docker context: {}", context.text);
        Some(context.text.trim().to_owned())
    })
    .as_deref()
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

/// Scenario: The local daemon. The CLI reaches the same daemon as [`runtime`].
async fn info_gives_linux_and_the_arch_the_cli_prints() {
    let output = docker(&["version", "--format", "{{.Server.Arch}}"], secs(60), true).await.unwrap();
    let arch = output.text.trim();
    assert!(!arch.is_empty(), "docker version prints the server's Arch");
    assert_eq!(runtime().info().await.unwrap(), RuntimeInfo::new(arch, "linux"));
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

fn dir(path: &str, mode: u32) -> CopyDir {
    let mut dir = CopyDir::new(path);
    dir.mode = mode;
    dir
}

/// The mode, owner and group of each folder the make_dirs tests look at.
const STAT_FOLDERS: &str = "stat -c '%n %a %u:%g' /tmp /run/postgresql";

async fn make_dirs_gives_user_1000_a_folder_and_leaves_existing_ones_alone() {
    // What the image holds before any call: /tmp at 1777, /run/postgresql at 3777 of postgres, /var/run a link.
    let before = spec("dirs-before", STAT_FOLDERS);
    let before_name = ran(&before).await;
    let before = runtime().logs(before_name, 100).await.unwrap();
    runtime().remove(before_name).await.unwrap();
    assert_eq!(before, "/tmp 1777 0:0\n/run/postgresql 3777 999:999\n", "the image changed");

    let script = format!(
        "touch /out/a && test -f /out/a && {STAT_FOLDERS} /out /tmp/out /run/app && test -L /var/run && echo link"
    );
    let mut spec = spec("dirs", &script);
    spec.user = Some("1000".into());
    runtime().create(&spec).await.unwrap();
    let dirs =
        [dir("/out", 0o777), CopyDir::new("/tmp/out"), dir("/run/postgresql", 0o700), CopyDir::new("/var/run/app")];
    runtime().make_dirs(&spec.name, &dirs).await.unwrap();
    runtime().make_dirs(&spec.name, &dirs).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    assert_eq!(ended(&spec.name).await, ContainerStatus::Exited { code: 0 }, "user 1000 writes /out/a");
    let logs = runtime().logs(&spec.name, 100).await.unwrap();
    let expected = format!("{before}/out 777 0:0\n/tmp/out 755 0:0\n/run/app 755 0:0\nlink\n");
    assert_eq!(logs, expected, "existing folders keep their owner and mode; /var/run/app is made through the link");
    runtime().remove(&spec.name).await.unwrap();
}

async fn make_dirs_refuses_a_file_and_a_path_below_a_file_and_a_late_call() {
    let spec = spec("dirs-file", "sleep 600");
    runtime().create(&spec).await.unwrap();
    // A file at the path, and below a file: this pins Docker's answer for a path below a file.
    for path in ["/etc/passwd", "/etc/passwd/x"] {
        let dirs = [CopyDir::new("/not-made"), CopyDir::new(path)];
        let error = runtime().make_dirs(&spec.name, &dirs).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Conflict, "{path}: {error}");
        assert!(error.message.contains("/etc/passwd is a file, not a folder"), "{error}");
    }
    let twice = [dir("/twice", 0o777), dir("/twice", 0o755)];
    assert_eq!(kind(runtime().make_dirs(&spec.name, &twice).await), ErrorKind::InvalidSpec);
    let bad_mode = [dir("/bad-mode", 0o10000)];
    assert_eq!(kind(runtime().make_dirs(&spec.name, &bad_mode).await), ErrorKind::InvalidSpec);
    runtime().start(&spec.name).await.unwrap();
    for path in ["/not-made", "/twice", "/bad-mode"] {
        assert!(!exists(&spec.name, path).await, "{path} not made");
    }
    let late = [CopyDir::new("/late")];
    assert_eq!(kind(runtime().make_dirs(&spec.name, &late).await), ErrorKind::Conflict, "after start");
    assert!(!exists(&spec.name, "/late").await, "nothing made");
    runtime().remove(&spec.name).await.unwrap();
    assert_eq!(kind(runtime().make_dirs(&spec.name, &late).await), ErrorKind::Conflict, "a missing container");
}

async fn make_dirs_on_a_read_only_root_makes_only_on_a_mount() {
    let volume = labelled_volume("ro-dirs").await;
    let mut spec = spec("ro-dirs", "stat -c '%n %a %u:%g' /work/cache; test -e /etc/x || echo no /etc/x");
    spec.read_only_root = true;
    spec.mounts.push(Mount::new(MountKind::Volume, &volume, "/work"));
    runtime().create(&spec).await.unwrap();
    let both = [dir("/work/cache", 0o777), CopyDir::new("/etc/x")];
    assert_eq!(kind(runtime().make_dirs(&spec.name, &both).await), ErrorKind::InvalidSpec);
    let root = [CopyDir::new("/work")];
    assert_eq!(kind(runtime().make_dirs(&spec.name, &root).await), ErrorKind::InvalidSpec, "the volume's root");
    runtime().make_dirs(&spec.name, &both[..1]).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    assert_eq!(ended(&spec.name).await, ContainerStatus::Exited { code: 0 });
    let logs = runtime().logs(&spec.name, 100).await.unwrap();
    assert_eq!(logs, "/work/cache 777 0:0\nno /etc/x\n");
    runtime().remove(&spec.name).await.unwrap();
}

/// Room for every file of a test.
fn roomy() -> CopyLimits {
    CopyLimits::new(100, 1 << 20)
}

/// A kept file as path, contents and mode; a skipped entry as path and reason.
type Kept<'a> = (&'a str, &'a [u8], u32);
type Left<'a> = (&'a str, &'a str);

/// The kept files and the skipped entries, each sorted: the order of Docker's archive may differ between engine
/// versions.
fn sorted(out: &CopyOut) -> (Vec<Kept<'_>>, Vec<Left<'_>>) {
    let mut files: Vec<_> = out.files.iter().map(|f| (f.path.as_str(), f.contents.as_slice(), f.mode)).collect();
    let mut skipped: Vec<_> = out.skipped.iter().map(|s| (s.path.as_str(), s.reason.as_str())).collect();
    files.sort();
    skipped.sort();
    (files, skipped)
}

/// Creates and starts `spec`, waits until it exits with code 0, and returns its name.
async fn ran(spec: &ContainerSpec) -> &str {
    runtime().create(spec).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    assert_eq!(ended(&spec.name).await, ContainerStatus::Exited { code: 0 });
    &spec.name
}

async fn copy_out_collects_regular_files_and_skips_links() {
    let script = "mkdir -p /out/sub && printf ay > /out/a && printf bee > /out/sub/b && chmod 755 /out/sub/b \
        && ln -s /etc/passwd /out/out && ln -s a /out/in";
    let spec = spec("copy-out", script);
    let name = ran(&spec).await;
    let out = runtime().copy_out(name, "/out", roomy()).await.unwrap();
    assert_eq!(
        sorted(&out),
        (
            vec![("/out/a", &b"ay"[..], 0o644), ("/out/sub/b", &b"bee"[..], 0o755)],
            vec![
                ("/out/in", "a symbolic link to a, not followed"),
                ("/out/out", "a symbolic link to /etc/passwd, not followed"),
            ]
        )
    );
    let one = runtime().copy_out(name, "/out/./sub/b", roomy()).await.unwrap();
    assert_eq!(sorted(&one), (vec![("/out/sub/b", &b"bee"[..], 0o755)], vec![]), "one file by its path");
    assert_eq!(runtime().copy_out(name, "/missing", roomy()).await.unwrap(), CopyOut::default());
    runtime().remove(name).await.unwrap();
}

async fn copy_out_keeps_within_its_limits() {
    let script = "mkdir -p /sizes /five /two && head -c 10 /dev/zero > /sizes/a && head -c 5000 /dev/zero > /sizes/b \
        && head -c 20 /dev/zero > /sizes/c && for i in 1 2 3 4 5; do echo $i > /five/$i; done \
        && echo 1 > /two/1 && echo 2 > /two/2";
    let spec = spec("copy-limits", script);
    let name = ran(&spec).await;
    let out = runtime().copy_out(name, "/sizes", CopyLimits::new(10, 100)).await.unwrap();
    let mut sizes: Vec<usize> = out.files.iter().map(|f| f.contents.len()).collect();
    sizes.sort();
    assert_eq!(sizes, [10, 20], "the small ones, whole");
    assert_eq!(out.skipped.len(), 1, "{out:?}");
    assert_eq!(out.skipped[0].path, "/sizes/b");
    assert!(out.skipped[0].reason.starts_with("5000 bytes, more than the "), "{out:?}");

    let out = runtime().copy_out(name, "/five", CopyLimits::new(2, 100)).await.unwrap();
    assert_eq!(out.files.len(), 2, "{out:?}");
    assert_eq!(sorted(&out).1, [("/five", "more than 2 files; the rest was not read")]);
    let out = runtime().copy_out(name, "/two", CopyLimits::new(2, 100)).await.unwrap();
    assert_eq!((out.files.len(), out.skipped.len()), (2, 0), "exactly at the limit: {out:?}");
    for zero in [CopyLimits::new(0, 100), CopyLimits::new(2, 0)] {
        assert_eq!(kind(runtime().copy_out(name, "/two", zero).await), ErrorKind::InvalidSpec, "{zero:?}");
    }
    runtime().remove(name).await.unwrap();
}

async fn copy_out_after_the_end_reads_a_volume_and_not_a_tmpfs() {
    let volume = labelled_volume("copy-out-vol").await;
    let mut spec = spec("copy-out-end", "printf vee > /vol/v && printf ess > /scratch/s && printf root > /r");
    spec.mounts.push(Mount::new(MountKind::Volume, &volume, "/vol"));
    spec.tmpfs.push(Tmpfs::new("/scratch"));
    let name = ran(&spec).await;
    let on_volume = runtime().copy_out(name, "/vol", roomy()).await.unwrap();
    assert_eq!(sorted(&on_volume).0, [("/vol/v", &b"vee"[..], 0o644)]);
    let on_root = runtime().copy_out(name, "/r", roomy()).await.unwrap();
    assert_eq!(sorted(&on_root).0, [("/r", &b"root"[..], 0o644)]);
    let on_tmpfs = runtime().copy_out(name, "/scratch", roomy()).await.unwrap();
    assert!(on_tmpfs.files.iter().all(|f| f.path != "/scratch/s"), "gone with the tmpfs: {on_tmpfs:?}");
    assert_eq!(runtime().copy_out(name, "/scratch/s", roomy()).await.unwrap(), CopyOut::default());
    runtime().remove(name).await.unwrap();
}

async fn copy_out_reads_a_progress_file_whole_while_it_is_renamed() {
    let script = r#"mkdir -p /work; pad=$(printf '%2000s' | tr ' ' x); i=0
        while true; do i=$((i+1)); printf '{"step":%d,"pad":"%s"}' $i "$pad" > /work/.progress.tmp
        mv /work/.progress.tmp /work/progress.json; done"#;
    let spec = spec("copy-out-progress", script);
    runtime().create(&spec).await.unwrap();
    runtime().start(&spec.name).await.unwrap();
    let limits = CopyLimits::new(1, 64 << 10);
    let mut steps = Vec::new();
    for _ in 0..10 {
        let out = eventually("a progress file", secs(60), async || {
            let out = runtime().copy_out(&spec.name, "/work/progress.json", limits).await?;
            Ok((!out.files.is_empty()).then_some(out))
        })
        .await;
        assert_eq!((out.files.len(), out.skipped.len()), (1, 0), "{out:?}");
        let progress: serde_json::Value = serde_json::from_slice(&out.files[0].contents).expect("one whole file");
        assert_eq!(progress["pad"].as_str().map(str::len), Some(2000));
        steps.push(progress["step"].as_u64().unwrap());
    }
    assert!(steps.windows(2).all(|w| w[0] <= w[1]), "{steps:?}");
    runtime().remove(&spec.name).await.unwrap();
}

async fn copy_out_reads_copy_in_back_and_refuses_what_is_not_there() {
    let spec = spec("copy-out-created", "echo never");
    runtime().create(&spec).await.unwrap();
    let mut file = CopyFile::new("/in/config.json", "{}");
    file.mode = 0o600;
    runtime().copy_in(&spec.name, std::slice::from_ref(&file)).await.unwrap();
    let out = runtime().copy_out(&spec.name, "/in/config.json", roomy()).await.unwrap();
    assert_eq!(out, CopyOut::new(vec![file], Vec::new()), "from a created container");
    assert_eq!(kind(runtime().copy_out(&name("copy-out-missing"), "/in", roomy()).await), ErrorKind::Conflict);
    for bad in ["in", "/", "/in/../etc"] {
        assert_eq!(kind(runtime().copy_out(&spec.name, bad, roomy()).await), ErrorKind::InvalidSpec, "{bad}");
    }
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

/// The user, password and `htpasswd -Bbn` line of the private registry.
const REGISTRY_USER: &str = "cicada-e2e";
const REGISTRY_PASSWORD: &str = "e2e-registry-pass";
const REGISTRY_HTPASSWD: &str = "cicada-e2e:$2y$05$OdcV5C9vc02VmEMF77aetOPuSPfGHHe8B.YdDSkTSx5c/kZS8lLsa";

/// A `registry:2` with basic auth on a free port of the host, a tag of [`PULLED`] in it, and two config folders of
/// its own: `login`, where the CLI logs in, and `empty`. Dropping it removes the registry, the tag and the folders;
/// the registry also carries the suite's label for the sweep.
struct PrivateRegistry {
    container: String,
    /// `localhost:<port>`.
    host: String,
    /// `<host>/cicada-rs-e2e/private:1`.
    image: String,
    folders: PathBuf,
}

impl PrivateRegistry {
    async fn start() -> Self {
        let container = name("registry");
        let folders = std::env::temp_dir().join(format!("cicada-rs-e2e-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folders);
        for folder in ["login", "empty"] {
            std::fs::create_dir_all(folders.join(folder)).unwrap();
        }
        docker(&["rm", "-f", &container], secs(60), false).await.unwrap();
        // The guard exists before the registry, so a failed start still cleans up.
        let mut registry = Self { container, host: String::new(), image: String::new(), folders };
        let label = format!("{}={}", CONTAINER_LABEL.0, CONTAINER_LABEL.1);
        let htpasswd = format!("HTPASSWD={REGISTRY_HTPASSWD}");
        let script = "mkdir -p /auth && printf '%s\\n' \"$HTPASSWD\" > /auth/htpasswd \
                      && exec /entrypoint.sh /etc/docker/registry/config.yml";
        let run = [
            "run",
            "-d",
            "--name",
            &registry.container,
            "--label",
            &label,
            "-p",
            "127.0.0.1::5000",
            "-e",
            "REGISTRY_AUTH=htpasswd",
            "-e",
            "REGISTRY_AUTH_HTPASSWD_REALM=cicada-e2e",
            "-e",
            "REGISTRY_AUTH_HTPASSWD_PATH=/auth/htpasswd",
            "-e",
            &htpasswd,
            "--entrypoint",
            "sh",
            "registry:2",
            "-c",
            script,
        ];
        docker(&run, secs(120), true).await.unwrap();
        let port = docker(&["port", &registry.container, "5000/tcp"], secs(60), true).await.unwrap();
        let port = port.text.lines().next().and_then(|line| line.rsplit(':').next()).unwrap().trim().to_owned();
        registry.host = format!("localhost:{port}");
        registry.image = format!("{}/cicada-rs-e2e/private:1", registry.host);
        registry
    }

    fn folder(&self, name: &str) -> PathBuf {
        self.folders.join(name)
    }

    /// `docker --config <login> [-H <context host>] <args>`: the CLI with the registry's own config folder, never the
    /// developer's `~/.docker`. That folder holds no context, so the daemon's endpoint is given.
    async fn cli(&self, args: &[&str]) -> cicada_e2e::docker::Output {
        let login = self.folder("login");
        let mut all = vec!["--config", login.to_str().unwrap()];
        if let Some(host) = context_host() {
            all.extend(["-H", host]);
        }
        all.extend(args);
        docker(&all, secs(120), false).await.unwrap()
    }

    /// Logs in with [`Self::cli`] once the registry answers, and pushes [`Self::image`]. The login lands in
    /// `login/config.json`: it starts with an entry for the registry, so the CLI keeps the login in the file instead of
    /// the platform's default helper (the keychain on macOS).
    async fn login_and_push(&self) {
        let config = self.folder("login").join("config.json");
        std::fs::write(&config, serde_json::json!({ "auths": { &self.host: {} } }).to_string()).unwrap();
        eventually(&format!("a login to {}", self.host), secs(60), async || {
            let output = self.cli(&["login", &self.host, "-u", REGISTRY_USER, "-p", REGISTRY_PASSWORD]).await;
            if output.ok() { Ok(Some(())) } else { Err(anyhow::anyhow!("{}", output.text)) }
        })
        .await;
        let written: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
        assert!(written["auths"][&self.host]["auth"].is_string(), "the login is in the file: {written}");
        assert!(written.get("credsStore").is_none(), "no helper: {written}");

        docker(&["pull", PULLED], secs(120), true).await.unwrap();
        docker(&["tag", PULLED, &self.image], secs(60), true).await.unwrap();
        let push = self.cli(&["push", &self.image]).await;
        assert!(push.ok(), "push {}: {}", self.image, push.text);
        docker(&["image", "rm", &self.image], secs(60), true).await.unwrap();
    }
}

impl Drop for PrivateRegistry {
    fn drop(&mut self) {
        if !self.image.is_empty() {
            docker_blocking(&["image", "rm", "-f", &self.image]);
        }
        docker_blocking(&["rm", "-f", "-v", &self.container]);
        let _ = std::fs::remove_dir_all(&self.folders);
    }
}

/// A config folder of its own under the temp folder, removed when dropped.
struct TempFolder(PathBuf);

impl TempFolder {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!("cicada-rs-e2e-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A backend that reads its login from `folder` instead of the developer's `~/.docker`.
fn reading(folder: &Path) -> DockerRuntime {
    connect().docker_config(folder)
}

/// A container `name(suffix)` of `image` that runs `true`, created by `runtime`.
async fn create_from(
    runtime: &DockerRuntime,
    suffix: &str,
    image: &str,
) -> (ContainerSpec, Result<(), ContainerError>) {
    let mut spec = spec(suffix, "true");
    spec.image = image.into();
    let result = runtime.create(&spec).await;
    (spec, result)
}

/// Scenario: A public image with a broken helper. The runtime reads its own folder, whose `credsStore` names a helper
/// that is not installed, and still pulls the public image.
async fn a_public_image_is_pulled_past_a_helper_that_is_not_installed() {
    let folder = TempFolder::new("broken-helper");
    std::fs::write(folder.0.join("config.json"), r#"{"credsStore": "cicada-no-such-helper"}"#).unwrap();
    docker(&["image", "rm", "-f", PULLED], secs(60), false).await.unwrap();
    assert!(docker_inspect("{{.Id}}", PULLED).await.is_none(), "{PULLED} is not local");

    let (spec, created) = create_from(&reading(&folder.0), "broken-helper", PULLED).await;
    created.unwrap();
    assert_eq!(status(&spec.name).await, ContainerStatus::Created);
    assert!(docker_inspect("{{.Id}}", PULLED).await.is_some(), "pulled");
    runtime().remove(&spec.name).await.unwrap();
}

/// Scenarios: A login made after connect; A private image after docker login; A private image without a login.
async fn a_private_image_is_pulled_with_the_login_of_config_json() {
    let registry = PrivateRegistry::start().await;
    // Made, and talking to the daemon, before the login folder has a `config.json`.
    let after_login = reading(&registry.folder("login"));
    after_login.info().await.unwrap();
    assert!(!registry.folder("login").join("config.json").exists());
    registry.login_and_push().await;

    let (spec, created) = create_from(&after_login, "private", &registry.image).await;
    created.unwrap();
    assert_eq!(status(&spec.name).await, ContainerStatus::Created);
    assert!(docker_inspect("{{.Id}}", &registry.image).await.is_some(), "pulled");
    runtime().remove(&spec.name).await.unwrap();
    docker(&["image", "rm", &registry.image], secs(60), true).await.unwrap();

    let (spec, created) = create_from(&reading(&registry.folder("empty")), "private-no-login", &registry.image).await;
    let error = created.unwrap_err();
    println!("a pull from {} without a login: {:?}, {}", registry.host, error.kind, error.message);
    assert!(matches!(error.kind, ErrorKind::ImageNotFound | ErrorKind::Other), "{error}");
    assert!(error.message.ends_with(&format!("(no credentials for {})", registry.host)), "{error}");
    assert!(!error.message.contains(REGISTRY_PASSWORD), "{error}");
    assert_eq!(status(&spec.name).await, ContainerStatus::Missing);
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
