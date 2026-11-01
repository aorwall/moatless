//! Names the processes holding memory as the container nears its cgroup cap.
//!
//! An OOM kill takes the whole container, so these lines in the previous
//! container's log are the only record of which process held the memory: the
//! sandbox, the agent CLI, or a build the agent ran.

use crate::pod_processes::{ProcessMemory, largest_by_rss};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const MEMORY_STAT_PATH: &str = "/sys/fs/cgroup/memory.stat";

/// An 8 GiB sandbox has been seen to fill at about 2.3 GiB a minute, which
/// crosses a 10% band every 20 s.
const POLL: Duration = Duration::from_secs(5);

/// The lowest band logged; each 10% above it logs once on the way up.
const FIRST_BAND_PERCENT: u64 = 70;

const PROCESSES_NAMED: usize = 5;

/// Start the watch, or `None` when the cgroup names no memory cap.
pub fn spawn(cancel: CancellationToken) -> Option<JoinHandle<()>> {
    let limit = crate::limits::memory_limit_bytes()?;
    Some(tokio::spawn(watch(limit, cancel)))
}

async fn watch(limit: u64, cancel: CancellationToken) {
    let mut reported = None;
    let mut tick = tokio::time::interval(POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            _ = tick.tick() => {}
        }
        let Some(anon) = std::fs::read_to_string(MEMORY_STAT_PATH)
            .ok()
            .and_then(|stat| anon_bytes(&stat))
        else {
            continue;
        };
        if let Some(percent) = crossed(&mut reported, band(anon, limit)) {
            tracing::warn!(
                percent,
                anon_mib = anon / MIB,
                limit_mib = limit / MIB,
                largest = %describe(&largest_by_rss(PROCESSES_NAMED)),
                "Container memory nearing its limit"
            );
        }
    }
}

/// The band to log when `now` rises above the high-water mark `reported`,
/// which only a drop under [`FIRST_BAND_PERCENT`] clears. Re-arming on every
/// lower band would log each poll that `anon` crosses back over a boundary.
fn crossed(reported: &mut Option<u64>, now: Option<u64>) -> Option<u64> {
    let rose = now > *reported;
    *reported = now.and((*reported).max(now));
    rose.then_some(now).flatten()
}

const MIB: u64 = 1024 * 1024;

/// The cgroup's `anon` bytes, rather than `memory.current`: page cache counts
/// toward `memory.current` and sits near the cap after any large build, but
/// the kernel reclaims it before it kills, so only anonymous memory predicts
/// an OOM.
fn anon_bytes(stat: &str) -> Option<u64> {
    stat.lines()
        .find_map(|line| line.strip_prefix("anon "))?
        .trim()
        .parse()
        .ok()
}

/// The 10% band `used` falls in, `None` below [`FIRST_BAND_PERCENT`].
fn band(used: u64, limit: u64) -> Option<u64> {
    let percent = used.saturating_mul(100) / limit.max(1);
    (percent >= FIRST_BAND_PERCENT).then_some(percent.min(100) / 10 * 10)
}

/// `pid:comm:MiB` per process, largest first. The comm only, never the
/// command line, which can carry a secret.
fn describe(processes: &[ProcessMemory]) -> String {
    processes
        .iter()
        .map(|p| format!("{}:{}:{}", p.pid, p.comm, p.rss_bytes / MIB))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * MIB;

    /// The bands [`crossed`] logs for `samples` MiB against an 8 GiB cap.
    fn logged(samples: &[u64]) -> Vec<u64> {
        let mut reported = None;
        samples
            .iter()
            .filter_map(|mib| crossed(&mut reported, band(mib * MIB, 8 * GIB)))
            .collect()
    }

    #[test]
    fn a_ramp_reports_each_band_once_until_it_drops_under_the_first() {
        let samples = [1233, 3295, 6000, 6500, 7000, 7882, 7500, 3000, 6000];
        assert_eq!(logged(&samples), [70, 80, 90, 70]);
    }

    /// 6500 MiB is 79% and 6600 MiB 80%: hovering there logs 80 once.
    #[test]
    fn hovering_at_a_band_boundary_logs_it_once() {
        assert_eq!(logged(&[6500, 6600, 6500, 6600, 6500, 6600]), [70, 80]);
    }

    #[test]
    fn anon_is_read_from_memory_stat_rather_than_its_prefixed_neighbours() {
        let stat = "anon_thp 0\nanon 6308233216\nfile 2147483648\n";
        assert_eq!(anon_bytes(stat), Some(6_308_233_216));
        assert_eq!(anon_bytes("file 1\n"), None);
    }

    #[test]
    fn a_process_reads_as_pid_comm_and_mib() {
        let processes = [
            ProcessMemory {
                pid: 812,
                comm: "rustc".into(),
                rss_bytes: 5 * GIB,
            },
            ProcessMemory {
                pid: 1,
                comm: "sandbox".into(),
                rss_bytes: 200 * MIB,
            },
        ];
        assert_eq!(describe(&processes), "812:rustc:5120, 1:sandbox:200");
    }
}
