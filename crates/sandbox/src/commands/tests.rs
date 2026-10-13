use super::*;

/// Whether `pid` is a live process. A killed orphan reparents to PID 1,
/// which does not reap in this container, so it lingers as a zombie and any
/// `kill -0` probe would report it alive forever.
fn is_live(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    // The state letter follows the parenthesised comm, which may itself
    // contain spaces or parens.
    stat.rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .is_some_and(|state| !matches!(state, "Z" | "X" | "x"))
}

fn registry(timeout: Duration) -> CommandRegistry {
    let dir = std::env::temp_dir().join(format!("moat-cmd-test-{}", short_id()));
    CommandRegistry::new(timeout, dir, "task-1".to_string())
}

/// A registry whose liveness reports land in the returned backend, so a
/// test can read the running counts the registry published.
fn with_capture(
    timeout: Duration,
) -> (
    CommandRegistry,
    Arc<crate::events::test_backend::CapturingBackend>,
    CancellationToken,
) {
    use crate::backend_client::BackendClient;
    let backend = Arc::new(crate::events::test_backend::CapturingBackend::default());
    let cancel = CancellationToken::new();
    let (outbox, _drainer) = crate::outbox::EventOutbox::spawn(
        Arc::clone(&backend) as Arc<dyn BackendClient>,
        "task-1".to_string(),
        cancel.clone(),
    );
    let dir = std::env::temp_dir().join(format!("moat-cmd-live-{}", short_id()));
    let reg = CommandRegistry::new(timeout, dir, "task-1".to_string())
        .with_liveness(GrpcEventPublisher::new(outbox));
    (reg, backend, cancel)
}

/// The `runningCommands` counts the backend received, oldest first. Drains
/// through the outbox, so it polls until at least `want` have landed.
async fn published_counts(
    backend: &crate::events::test_backend::CapturingBackend,
    want: usize,
) -> Vec<i64> {
    for _ in 0..200 {
        let counts = counts_now(backend);
        if counts.len() >= want {
            return counts;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    counts_now(backend)
}

fn counts_now(backend: &crate::events::test_backend::CapturingBackend) -> Vec<i64> {
    backend
        .received
        .lock()
        .expect("lock")
        .iter()
        .filter(|r| r.sub_type == AgentSubType::CommandsChanged.as_str())
        .map(|r| {
            serde_json::from_str::<serde_json::Value>(&r.payload_json).expect("payload JSON")
                ["runningCommands"]
                .as_i64()
                .expect("count")
        })
        .collect()
}

/// A registry whose journal is a throwaway file, plus that file's path.
fn journalled(timeout: Duration) -> (CommandRegistry, PathBuf) {
    let dir = std::env::temp_dir().join(format!("moat-cmd-journal-{}", short_id()));
    std::fs::create_dir_all(&dir).expect("journal dir");
    let path = dir.join(".moat-commands.json");
    (
        CommandRegistry::new(timeout, dir, "task-1".to_string()).with_journal(path.clone()),
        path,
    )
}

/// A shared checkout gives two Members one `WORKSPACE_DIR`, and each take
/// deletes the file it read, so one Member's recovery must leave the
/// other's record of what the restart destroyed.
#[tokio::test]
async fn one_members_take_leaves_the_other_members_journal() {
    let dir = std::env::temp_dir().join(format!("moat-cmd-shared-{}", short_id()));
    tokio::fs::create_dir_all(&dir).await.expect("journal dir");
    let workspace = dir.display().to_string();
    let entry = |id: &str| JournaledCommand {
        id: id.to_string(),
        command: "bash /tmp/verify.sh".to_string(),
        label: "verify".to_string(),
        started_at_unix_ms: 1,
        state: CommandState::Running,
        exit_code: None,
        log_tail: None,
    };
    for (task_id, id) in [("task-a", "aaaaaaaa"), ("task-b", "bbbbbbbb")] {
        tokio::fs::write(
            default_journal_path(&workspace, task_id),
            serde_json::to_vec(&[entry(id)]).expect("encode"),
        )
        .await
        .expect("seed");
    }

    let taken = take_journal(&default_journal_path(&workspace, "task-a")).await;
    assert_eq!(taken, vec![entry("aaaaaaaa")]);
    assert_eq!(
        journal_now(&default_journal_path(&workspace, "task-b")).await,
        vec![entry("bbbbbbbb")]
    );
}

async fn journal_now(path: &Path) -> Vec<JournaledCommand> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).expect("journal parses"),
        Err(_) => Vec::new(),
    }
}

async fn wait_terminal(reg: &CommandRegistry, id: &str) -> CommandRecord {
    for _ in 0..200 {
        let cmd = reg.get(id).await.expect("command registered");
        if cmd.state.is_terminal() {
            return cmd;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("command never reached a terminal state");
}

/// Stamped per Member; see `crates/sandbox/AGENTS.md` "Spawning a child
/// process".
#[tokio::test]
async fn a_command_sees_the_registrys_own_task_id() {
    let dir = std::env::temp_dir().join(format!("moat-cmd-env-{}", short_id()));
    let reg = CommandRegistry::new(Duration::from_mins(1), dir, "fork-task".to_string());

    let cmd = reg
        .run(CommandSpec {
            command: "printf %s \"$TASK_ID\"".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    let done = wait_terminal(&reg, &cmd.id).await;
    assert_eq!(done.exit_code, Some(0));

    let log = tokio::fs::read_to_string(&cmd.log_path)
        .await
        .expect("command log");
    assert_eq!(log.trim(), "fork-task", "log was {log:?}");
}

/// The writer a registry entry stops naming: a command that backgrounds a
/// process and exits leaves it running, which is why certification reads the
/// pod's live processes and not this registry.
#[tokio::test]
async fn a_process_the_command_orphaned_outlives_the_registry_entry() {
    let reg = registry(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "(sleep 30 & echo $!) ; exit 0".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    let done = wait_terminal(&reg, &cmd.id).await;
    assert_eq!(done.exit_code, Some(0));
    let orphan: u32 = tokio::fs::read_to_string(&cmd.log_path)
        .await
        .expect("command log")
        .trim()
        .parse()
        .expect("the backgrounded pid");

    assert!(!reg.busy().await, "the command's own entry went terminal");
    let live = crate::pod_processes::foreign_live_processes().expect("/proc is readable");
    assert!(
        live.iter().any(|p| p.pid == orphan),
        "the orphaned process is missing from {live:?}"
    );

    signal_group(orphan, "KILL").await;
}

#[tokio::test]
async fn publishes_the_running_count_when_a_command_starts_and_finishes() {
    let (reg, backend, cancel) = with_capture(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "true".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    wait_terminal(&reg, &cmd.id).await;

    // One at start (1 running) and one at exit (0), in that order.
    let counts = published_counts(&backend, 2).await;
    assert_eq!(counts, vec![1, 0], "got {counts:?}");
    cancel.cancel();
}

#[tokio::test]
async fn a_fresh_registry_publishes_its_zero_so_a_stale_row_is_cleared() {
    let (reg, backend, cancel) = with_capture(Duration::from_mins(1));
    reg.report_initial_liveness().await;

    let counts = published_counts(&backend, 1).await;
    assert_eq!(counts, vec![0], "a booting pod owns no command");
    cancel.cancel();
}

#[tokio::test]
async fn records_exit_code_and_log() {
    let reg = registry(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "echo hello; exit 7".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    assert_eq!(cmd.state, CommandState::Running);

    let done = wait_terminal(&reg, &cmd.id).await;
    assert_eq!(done.state, CommandState::Exited);
    assert_eq!(done.exit_code, Some(7));
    let log = std::fs::read_to_string(&done.log_path).expect("log written");
    assert!(log.contains("hello"), "log was {log:?}");
}

#[tokio::test]
async fn busy_only_while_running() {
    let reg = registry(Duration::from_mins(1));
    assert!(!reg.busy().await, "empty registry is not busy");
    let cmd = reg
        .run(CommandSpec {
            command: "true".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    wait_terminal(&reg, &cmd.id).await;
    assert!(
        !reg.busy().await,
        "finished command must not defer the reaper"
    );
}

#[tokio::test]
async fn timeout_kills_and_marks_timed_out() {
    let reg = registry(Duration::from_millis(200));
    let cmd = reg
        .run(CommandSpec {
            command: "sleep 60".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");

    let done = wait_terminal(&reg, &cmd.id).await;
    assert_eq!(done.state, CommandState::TimedOut);
    assert_eq!(done.exit_code, Some(TIMEOUT_EXIT_CODE));
    assert!(
        !reg.busy().await,
        "a timed-out command stops deferring the reaper"
    );
}

#[tokio::test]
async fn per_command_timeout_overrides_the_default() {
    // The default is long; the command's own value is what applies.
    let reg = registry(Duration::from_hours(1));
    let cmd = reg
        .run(CommandSpec {
            command: "sleep 60".into(),
            cwd: None,
            timeout: Some(Duration::from_millis(200)),
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    let done = wait_terminal(&reg, &cmd.id).await;
    assert_eq!(done.state, CommandState::TimedOut);
}

#[tokio::test]
async fn kill_marks_killed() {
    let reg = registry(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "sleep 60".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    reg.kill(&cmd.id).await.expect("kill");
    let done = wait_terminal(&reg, &cmd.id).await;
    assert_eq!(done.state, CommandState::Killed);
}

fn spec(command: &str, queue: bool) -> CommandSpec {
    CommandSpec {
        command: command.into(),
        cwd: None,
        timeout: None,
        label: None,
        queue,
    }
}

/// The incident `--queue` exists for: a second build must not start while
/// the first holds the shared target directory.
#[tokio::test]
async fn queued_commands_run_one_at_a_time_in_order() {
    let reg = registry(Duration::from_mins(1));
    let first = reg.run(spec("sleep 0.5", true)).await.expect("first");
    let second = reg.run(spec("true", true)).await.expect("second");
    let third = reg.run(spec("true", true)).await.expect("third");
    assert_eq!(second.state, CommandState::Queued);
    assert_eq!(second.pid, 0, "a queued command has no process yet");
    assert!(reg.busy().await, "a queued command keeps the sandbox awake");

    let first = wait_terminal(&reg, &first.id).await;
    let second = wait_terminal(&reg, &second.id).await;
    let third = wait_terminal(&reg, &third.id).await;
    assert!(second.started_at_unix_ms >= first.finished_at_unix_ms);
    assert!(third.started_at_unix_ms >= second.finished_at_unix_ms);
    assert_eq!(third.exit_code, Some(0));
}

/// The gRPC server runs on the multi-threaded runtime, where spawned tasks are
/// first polled in no fixed order; the order must not depend on that.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queued_commands_keep_registration_order_across_worker_threads() {
    let reg = registry(Duration::from_mins(1));
    let out = std::env::temp_dir().join(format!("moat-cmd-order-{}", short_id()));
    let mut ids = Vec::new();
    for i in 0..20 {
        let cmd = spec(&format!("echo {i} >> {}", out.display()), true);
        ids.push(reg.run(cmd).await.expect("queued").id);
    }
    for id in &ids {
        wait_terminal(&reg, id).await;
    }
    let order: Vec<String> = std::fs::read_to_string(&out)
        .expect("output")
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_file(&out);
    let expected: Vec<String> = (0..20).map(|i| i.to_string()).collect();
    assert_eq!(order, expected);
}

/// Killing a waiting command must not let the one behind it overlap the
/// command still running ahead of both.
#[tokio::test]
async fn killing_a_queued_command_keeps_the_next_behind_the_running_one() {
    let reg = registry(Duration::from_mins(1));
    let running = reg.run(spec("sleep 0.5", true)).await.expect("running");
    let killed = reg.run(spec("true", true)).await.expect("killed");
    let next = reg.run(spec("true", true)).await.expect("next");
    reg.kill(&killed.id).await.expect("kill");

    let running = wait_terminal(&reg, &running.id).await;
    let next = wait_terminal(&reg, &next.id).await;
    assert!(next.started_at_unix_ms >= running.finished_at_unix_ms);
}

#[tokio::test]
async fn a_command_without_queue_starts_while_the_queue_is_busy() {
    let reg = registry(Duration::from_mins(1));
    let queued = reg.run(spec("sleep 30", true)).await.expect("queued");
    let direct = reg.run(spec("true", false)).await.expect("direct");
    assert_eq!(direct.state, CommandState::Running);
    assert_eq!(wait_terminal(&reg, &direct.id).await.exit_code, Some(0));
    reg.kill(&queued.id).await.expect("kill");
}

/// A failed build must not strand the commands queued behind it.
#[tokio::test]
async fn a_failed_queued_command_releases_the_queue() {
    let reg = registry(Duration::from_mins(1));
    let failing = reg.run(spec("exit 3", true)).await.expect("failing");
    let next = reg.run(spec("true", true)).await.expect("next");
    assert_eq!(wait_terminal(&reg, &failing.id).await.exit_code, Some(3));
    assert_eq!(wait_terminal(&reg, &next.id).await.exit_code, Some(0));
}

#[tokio::test]
async fn killing_a_queued_command_never_starts_it_and_frees_its_place() {
    let reg = registry(Duration::from_mins(1));
    let running = reg.run(spec("sleep 0.5", true)).await.expect("running");
    let killed = reg.run(spec("true", true)).await.expect("killed");
    let next = reg.run(spec("true", true)).await.expect("next");

    reg.kill(&killed.id).await.expect("kill");
    let killed = wait_terminal(&reg, &killed.id).await;
    assert_eq!(killed.state, CommandState::Killed);
    assert_eq!(killed.pid, 0, "a command killed in the queue never spawned");
    wait_terminal(&reg, &running.id).await;
    assert_eq!(wait_terminal(&reg, &next.id).await.exit_code, Some(0));
}

/// A deadline counted from registration would kill a command for time it
/// spent waiting behind another.
#[tokio::test]
async fn a_queued_commands_deadline_starts_when_it_does() {
    let reg = registry(Duration::from_mins(1));
    reg.run(spec("sleep 1.5", true)).await.expect("first");
    let mut short = spec("true", true);
    short.timeout = Some(Duration::from_secs(1));
    let short = reg.run(short).await.expect("short");
    let short = wait_terminal(&reg, &short.id).await;
    assert_eq!(short.state, CommandState::Exited);
    assert_eq!(short.exit_code, Some(0));
}

#[tokio::test]
async fn kill_unknown_id_is_not_found() {
    let reg = registry(Duration::from_mins(1));
    assert!(matches!(
        reg.kill("nope").await,
        Err(CommandError::NotFound(_))
    ));
}

#[tokio::test]
async fn an_unknown_id_owes_nothing() {
    let reg = registry(Duration::from_mins(1));
    assert!(reg.unreported_among(&["gone".to_string()]).await.is_empty());
}

#[tokio::test]
async fn kill_reaches_the_whole_process_group() {
    let reg = registry(Duration::from_mins(1));
    // The wrapper exits immediately; the grandchild is what must die.
    let cmd = reg
        .run(CommandSpec {
            command: "sleep 60 & echo $!; wait".into(),
            cwd: None,
            timeout: Some(Duration::from_millis(200)),
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    let done = wait_terminal(&reg, &cmd.id).await;
    let grandchild: u32 = std::fs::read_to_string(&done.log_path)
        .expect("log")
        .trim()
        .parse()
        .expect("pid line");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !is_live(grandchild),
        "grandchild {grandchild} survived the group kill"
    );
}

#[tokio::test]
async fn a_finished_command_stays_unreported_until_the_agent_reads_it() {
    let reg = registry(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "exit 3".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    wait_terminal(&reg, &cmd.id).await;

    let owed = reg.unreported().await;
    assert_eq!(owed.len(), 1, "a finished command the agent never saw");

    reg.read_for_agent(None).await.expect("read");
    assert!(
        reg.unreported().await.is_empty(),
        "reading it back is the agent seeing it"
    );
}

/// Reading a *running* command tells the agent nothing about how it ends,
/// so it must not discharge the wake owed when it later fails.
#[tokio::test]
async fn reading_a_running_command_does_not_discharge_its_wake() {
    let reg = registry(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "sleep 0.2; exit 4".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");

    reg.read_for_agent(None).await.expect("read while running");
    wait_terminal(&reg, &cmd.id).await;

    let owed = reg.unreported().await;
    assert_eq!(owed.len(), 1, "still owes the agent the outcome");
}

#[tokio::test]
async fn killing_a_command_reports_it_without_a_wake() {
    let reg = registry(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "sleep 30".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    reg.kill(&cmd.id).await.expect("kill");
    wait_terminal(&reg, &cmd.id).await;

    assert!(
        reg.unreported().await.is_empty(),
        "the agent asked for this death; announcing it is noise"
    );
}

#[tokio::test]
async fn a_command_runs_in_the_requested_cwd() {
    let reg = registry(Duration::from_mins(1));
    let dir = std::env::temp_dir().join(format!("moat-cmd-cwd-{}", short_id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let cmd = reg
        .run(CommandSpec {
            command: "pwd".into(),
            cwd: Some(dir.display().to_string()),
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");

    let done = wait_terminal(&reg, &cmd.id).await;
    let log = std::fs::read_to_string(&done.log_path).expect("log");
    assert!(
        log.trim()
            .ends_with(&dir.file_name().unwrap().to_string_lossy().to_string()),
        "ran in {log:?}, wanted {}",
        dir.display()
    );
}

#[tokio::test]
async fn log_tail_reads_the_end_and_drops_the_partial_line() {
    let dir = std::env::temp_dir().join(format!("moat-cmd-tail-{}", short_id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("t.log");
    let body = (0..500)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, &body).expect("write");

    let tail = log_tail(&path, 3, 4096).expect("tail");
    assert_eq!(tail, "line 497\nline 498\nline 499");

    // Seeking into the middle of the file must not emit a half line.
    let tail = log_tail(&path, 500, 64).expect("tail");
    assert!(
        tail.lines().all(|l| l.starts_with("line ")),
        "tail was {tail:?}"
    );
}

#[tokio::test]
async fn log_tail_of_a_missing_or_empty_log_is_none() {
    let dir = std::env::temp_dir().join(format!("moat-cmd-tail-{}", short_id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let empty = dir.join("empty.log");
    std::fs::write(&empty, "").expect("write");

    assert!(log_tail(&dir.join("nope.log"), 5, 4096).is_none());
    assert!(log_tail(&empty, 5, 4096).is_none());
}

/// The mirror writes the backend has seen, oldest first. The send is
/// detached from the entries lock, so it polls until at least `want` have
/// landed.
async fn mirror_writes(
    backend: &crate::events::test_backend::CapturingBackend,
    want: usize,
) -> Vec<(i64, Vec<JournaledCommand>)> {
    for _ in 0..200 {
        let writes = backend.unreported_commands.lock().expect("lock").clone();
        if writes.len() >= want {
            return writes;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    backend.unreported_commands.lock().expect("lock").clone()
}

/// The failure the row exists for: a replacement whose workspace restore
/// fails has no journal, so the set must also reach the backend — with a
/// revision, because the send happens off the entries lock.
#[tokio::test]
async fn the_unreported_set_is_mirrored_to_the_backend_with_a_rising_revision() {
    let backend = Arc::new(crate::events::test_backend::CapturingBackend::default());
    let dir = std::env::temp_dir().join(format!("moat-cmd-mirror-{}", short_id()));
    let start = unix_now_ms();
    let reg = CommandRegistry::new(Duration::from_mins(1), dir, "t1".to_string())
        .with_backend_mirror(Arc::clone(&backend) as Arc<dyn BackendClient>);

    let cmd = reg
        .run(CommandSpec {
            command: "exit 4".into(),
            cwd: None,
            timeout: None,
            label: Some("verify".into()),
            queue: false,
        })
        .await
        .expect("spawn");
    wait_terminal(&reg, &cmd.id).await;

    // The spawn and the exit each mirror: running, then the outcome.
    let writes = mirror_writes(&backend, 2).await;
    assert!(writes.len() >= 2, "spawn and exit each write the mirror");
    // Seeded from the clock so it outruns a prior pod's stored revision,
    // never from 1.
    assert!(
        writes[0].0 >= start,
        "the first revision starts at the clock, not at zero"
    );
    assert_eq!(writes[0].1[0].state, CommandState::Running);
    let last = writes.last().expect("a write");
    assert!(last.0 > writes[0].0, "each write carries a higher revision");
    assert_eq!(last.1[0].id, cmd.id);
    assert_eq!(last.1[0].label, "verify");
    assert_eq!(last.1[0].state, CommandState::Exited);
    assert_eq!(last.1[0].exit_code, Some(4));

    // Reading it back marks it reported, so the row stops naming it.
    reg.read_for_agent(Some(&cmd.id)).await.expect("read back");
    let writes = mirror_writes(&backend, writes.len() + 1).await;
    assert!(
        writes.last().expect("a write").1.is_empty(),
        "a command the agent has been shown is owed nothing"
    );
}

/// The restart case: the pod dies with a command in flight. Its process and
/// log go with it, so the journal is the only thing a replacement pod can
/// read to know the agent is still owed an answer.
#[tokio::test]
async fn a_running_command_is_journalled_for_a_replacement_pod() {
    let (reg, path) = journalled(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "sleep 30".into(),
            cwd: None,
            timeout: None,
            label: Some("verify".into()),
            queue: false,
        })
        .await
        .expect("spawn");

    let journal = journal_now(&path).await;
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0].id, cmd.id);
    assert_eq!(journal[0].label, "verify");
    assert_eq!(journal[0].state, CommandState::Running);
}

/// Installed by renaming a sibling temp file over it, so a crash mid-write
/// never leaves a truncated file; the temp file must not survive a
/// successful write, or it rides the snapshot as untracked cruft.
#[tokio::test]
async fn writing_the_journal_leaves_no_temp_file() {
    let (reg, path) = journalled(Duration::from_mins(1));
    reg.run(CommandSpec {
        command: "sleep 30".into(),
        cwd: None,
        timeout: None,
        label: None,
        queue: false,
    })
    .await
    .expect("spawn");

    assert!(path.exists(), "the journal was written");
    assert!(
        !path.with_extension("json.tmp").exists(),
        "the staging file must be renamed away, not left behind"
    );
}

/// A finished command the agent never read still owes it an answer, so the
/// journal keeps it — with the outcome, which outlives the log.
#[tokio::test]
async fn a_finished_but_unread_command_stays_journalled_with_its_outcome() {
    let (reg, path) = journalled(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "exit 4".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    wait_terminal(&reg, &cmd.id).await;

    let journal = journal_now(&path).await;
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0].state, CommandState::Exited);
    assert_eq!(journal[0].exit_code, Some(4));
}

/// Reading the outcome back marks it reported, so a restart after that must
/// not re-announce a command the agent has already seen.
#[tokio::test]
async fn reading_a_command_back_clears_it_from_the_journal() {
    let (reg, path) = journalled(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "true".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    wait_terminal(&reg, &cmd.id).await;
    assert_eq!(journal_now(&path).await.len(), 1);

    reg.read_for_agent(None).await.expect("agent reads it back");
    assert!(
        journal_now(&path).await.is_empty(),
        "a reported command must not survive into the next pod"
    );
}

/// `moat cmd wait` reads a running command every few seconds; a read that
/// reports nothing must not rewrite the journal or call the backend.
#[tokio::test]
async fn reading_a_running_command_does_not_rewrite_the_journal() {
    let (reg, path) = journalled(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "sleep 30".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    tokio::fs::remove_file(&path)
        .await
        .expect("journal written");

    reg.read_for_agent(Some(&cmd.id)).await.expect("read");
    reg.read_for_agent(None).await.expect("read all");
    assert!(
        !path.exists(),
        "a read that reports nothing must not mirror"
    );
}

/// `kill` marks the command reported at the request, while the process
/// still winds down. The journal keys off `reported`, so it drops the
/// command then, not when the supervisor records the death.
#[tokio::test]
async fn killing_a_command_clears_it_from_the_journal() {
    let (reg, path) = journalled(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "sleep 30".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    assert_eq!(journal_now(&path).await.len(), 1);

    reg.kill(&cmd.id).await.expect("kill");
    assert!(journal_now(&path).await.is_empty());
    wait_terminal(&reg, &cmd.id).await;
    assert!(journal_now(&path).await.is_empty());
}

#[tokio::test]
async fn a_wake_clears_the_journal_so_a_restart_does_not_repeat_it() {
    let (reg, path) = journalled(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "true".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    wait_terminal(&reg, &cmd.id).await;

    reg.mark_reported(std::slice::from_ref(&cmd.id)).await;
    assert!(journal_now(&path).await.is_empty());
}

/// Reading the journal clears it, so the loss is announced once however
/// many times the pod restarts afterwards.
#[tokio::test]
async fn take_journal_reads_once_and_clears_the_file() {
    let (reg, path) = journalled(Duration::from_mins(1));
    reg.run(CommandSpec {
        command: "sleep 30".into(),
        cwd: None,
        timeout: None,
        label: None,
        queue: false,
    })
    .await
    .expect("spawn");

    assert_eq!(take_journal(&path).await.len(), 1);
    assert!(!path.exists(), "the journal must not be read twice");
    assert!(take_journal(&path).await.is_empty());
}

#[tokio::test]
async fn take_journal_of_a_missing_or_corrupt_file_is_empty() {
    let dir = std::env::temp_dir().join(format!("moat-cmd-journal-{}", short_id()));
    std::fs::create_dir_all(&dir).expect("dir");
    assert!(take_journal(&dir.join("absent.json")).await.is_empty());

    let corrupt = dir.join("corrupt.json");
    std::fs::write(&corrupt, b"{ not json").expect("write");
    assert!(take_journal(&corrupt).await.is_empty());
    assert!(
        !corrupt.exists(),
        "a journal that cannot be parsed must not be retried on every start"
    );
}

/// A registry with no journal configured must behave exactly as before.
#[tokio::test]
async fn a_registry_without_a_journal_writes_nothing() {
    let reg = registry(Duration::from_mins(1));
    let cmd = reg
        .run(CommandSpec {
            command: "true".into(),
            cwd: None,
            timeout: None,
            label: None,
            queue: false,
        })
        .await
        .expect("spawn");
    wait_terminal(&reg, &cmd.id).await;
    assert_eq!(reg.unreported().await.len(), 1);
}
