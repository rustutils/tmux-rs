//! The jobs of `#()` in formats (format.c's `format_job_tree`).
//!
//! A job is kept by the tag of the format tree that ran it and its raw
//! command. It remembers what the command last expanded to, its last
//! output and when it last ran. The store decides when to start a job
//! again and what to show meanwhile. Starting jobs, their callbacks and
//! the status line redraws are the adapter's (src/format.rs). There is
//! one store for jobs without a client (in the bridge) and one per client.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// One job, as C's struct `format_job` without its client and command.
#[derive(Debug)]
pub struct FormatJob<J> {
    /// What the command expanded to when it last ran.
    pub expanded: Option<Vec<u8>>,
    /// When it last ran, or last redrew the status line.
    pub last: i64,
    /// Its last output line, or a placeholder.
    pub out: Option<Vec<u8>>,
    /// Whether output arrived since it last started.
    pub updated: bool,
    /// The running job.
    pub job: Option<J>,
    /// Whether a status line shows it, so its output redraws it.
    pub status: bool,
}

/// What [`FormatJobs::get`] decided.
#[derive(Debug, PartialEq, Eq)]
pub struct JobDecision<J> {
    /// A running job to stop first.
    pub stop: Option<J>,
    /// Whether to start the job now. The caller reports how that went with
    /// [`FormatJobs::started`].
    pub start: bool,
}

/// The jobs of one owner, by tag and raw command.
#[derive(Debug)]
pub struct FormatJobs<J> {
    jobs: BTreeMap<(u32, Vec<u8>), FormatJob<J>>,
}

impl<J> Default for FormatJobs<J> {
    fn default() -> Self {
        Self { jobs: BTreeMap::new() }
    }
}

impl<J: Copy + PartialEq> FormatJobs<J> {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of jobs kept.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// Whether no job is kept.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// `format_job_get`'s decisions for `cmd` under `tag`, now that it
    /// expands to `expanded`: a job whose expansion changed starts again,
    /// as does every job when `force` (`FORMAT_FORCE`) is set, and one that
    /// is not running starts at most once a second. A job running more
    /// than a second without output shows that it is not ready.
    pub fn get(&mut self, tag: u32, cmd: &[u8], expanded: &[u8], now: i64, force: bool) -> JobDecision<J> {
        let fj = self.jobs.entry((tag, cmd.to_vec())).or_insert_with(|| FormatJob {
            expanded: None,
            last: 0,
            out: None,
            updated: false,
            job: None,
            status: false,
        });
        let force = if fj.expanded.as_deref() != Some(expanded) {
            fj.expanded = Some(expanded.to_vec());
            true
        } else {
            force
        };
        let stop = if force { fj.job.take() } else { None };
        if force || (fj.job.is_none() && fj.last != now) {
            return JobDecision { stop, start: true };
        }
        if fj.job.is_some() && now - fj.last > 1 && fj.out.is_none() {
            fj.out = Some(not_ready(cmd));
        }
        JobDecision { stop, start: false }
    }

    /// Records that the job for `cmd` under `tag` was started (`job` is
    /// Some) or failed to start, at `now`.
    pub fn started(&mut self, tag: u32, cmd: &[u8], job: Option<J>, now: i64) {
        let Some(fj) = self.jobs.get_mut(&(tag, cmd.to_vec())) else {
            debug_assert!(false, "started a job the store does not have");
            return;
        };
        if job.is_none() {
            let mut out = b"<'".to_vec();
            out.extend_from_slice(cmd);
            out.extend_from_slice(b"' didn't start>");
            fj.out = Some(out);
        }
        fj.job = job;
        fj.last = now;
        fj.updated = false;
    }

    /// Marks the job as shown in a status line, and returns its last
    /// output.
    pub fn output(&mut self, tag: u32, cmd: &[u8], status: bool) -> Option<Vec<u8>> {
        let fj = self.jobs.get_mut(&(tag, cmd.to_vec()))?;
        if status {
            fj.status = true;
        }
        fj.out.clone()
    }

    /// Whether `job` is one of this store's.
    pub fn has_job(&self, job: J) -> bool {
        self.jobs.values().any(|fj| fj.job == Some(job))
    }

    /// `format_job_update`: the last complete line of new output. Returns
    /// whether the status line should be redrawn (at most once a second).
    pub fn update(&mut self, job: J, line: Vec<u8>, now: i64) -> bool {
        let Some(fj) = self.jobs.values_mut().find(|fj| fj.job == Some(job)) else {
            return false;
        };
        fj.updated = true;
        fj.out = Some(line);
        if fj.status && fj.last != now {
            fj.last = now;
            return true;
        }
        false
    }

    /// `format_job_complete`: the job finished with `out` as its last line
    /// (or all its output). Output that is empty after an update keeps
    /// the update. Returns whether the status line should be redrawn.
    pub fn complete(&mut self, job: J, out: Vec<u8>) -> bool {
        let Some(fj) = self.jobs.values_mut().find(|fj| fj.job == Some(job)) else {
            return false;
        };
        fj.job = None;
        if !out.is_empty() || !fj.updated {
            fj.out = Some(out);
        }
        std::mem::take(&mut fj.status)
    }

    /// `format_job_tidy`: forgets jobs that did not run for an hour, or all
    /// of them with `all`. Returns the running jobs to stop.
    pub fn tidy(&mut self, now: i64, all: bool) -> Vec<J> {
        let mut stop = Vec::new();
        self.jobs.retain(|_, fj| {
            let keep = !all && !(fj.last <= now && now - fj.last >= 3600);
            if !keep && let Some(job) = fj.job {
                stop.push(job);
            }
            keep
        });
        stop
    }
}

/// The placeholder for a job that runs without output yet.
fn not_ready(cmd: &[u8]) -> Vec<u8> {
    let mut out = b"<'".to_vec();
    out.extend_from_slice(cmd);
    out.extend_from_slice(b"' not ready>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_once_then_waits() {
        let mut jobs = FormatJobs::<u32>::new();
        let d = jobs.get(0, b"date", b"date", 100, false);
        assert_eq!(d, JobDecision { stop: None, start: true });
        jobs.started(0, b"date", Some(7), 100);
        assert_eq!(jobs.get(0, b"date", b"date", 100, false), JobDecision { stop: None, start: false });
        assert_eq!(jobs.output(0, b"date", false), None);
        // Running for more than a second without output.
        jobs.get(0, b"date", b"date", 102, false);
        assert_eq!(jobs.output(0, b"date", false), Some(b"<'date' not ready>".to_vec()));
    }

    #[test]
    fn changed_expansion_restarts() {
        let mut jobs = FormatJobs::<u32>::new();
        jobs.get(0, b"echo #{x}", b"echo 1", 100, false);
        jobs.started(0, b"echo #{x}", Some(1), 100);
        let d = jobs.get(0, b"echo #{x}", b"echo 2", 100, false);
        assert_eq!(d, JobDecision { stop: Some(1), start: true });
    }

    #[test]
    fn failed_start() {
        let mut jobs = FormatJobs::<u32>::new();
        jobs.get(3, b"x", b"x", 5, false);
        jobs.started(3, b"x", None, 5);
        assert_eq!(jobs.output(3, b"x", false), Some(b"<'x' didn't start>".to_vec()));
    }

    #[test]
    fn output_and_status() {
        let mut jobs = FormatJobs::<u32>::new();
        jobs.get(0, b"c", b"c", 10, false);
        jobs.started(0, b"c", Some(4), 10);
        assert!(jobs.has_job(4));
        jobs.output(0, b"c", true);
        assert!(jobs.update(4, b"line".to_vec(), 11));
        assert!(!jobs.update(4, b"line2".to_vec(), 11));
        // Empty output after an update keeps the update.
        assert!(jobs.complete(4, Vec::new()));
        assert_eq!(jobs.output(0, b"c", false), Some(b"line2".to_vec()));
        assert!(!jobs.has_job(4));
    }

    #[test]
    fn tidy() {
        let mut jobs = FormatJobs::<u32>::new();
        jobs.get(0, b"old", b"old", 0, false);
        jobs.started(0, b"old", Some(1), 0);
        jobs.get(0, b"new", b"new", 4000, false);
        jobs.started(0, b"new", None, 4000);
        assert_eq!(jobs.tidy(4000, false), vec![1]);
        assert_eq!(jobs.len(), 1);
        assert!(jobs.tidy(4000, true).is_empty());
        assert!(jobs.is_empty());
    }
}
