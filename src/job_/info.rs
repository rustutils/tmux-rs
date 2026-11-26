// Copyright (c) 2009 Nicholas Marriott <nicholas.marriott@gmail.com>
//
// Permission to use, copy, modify, and distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
// IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
// OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

//! What a job is and the decisions about it, from job.c of tmux 3.8 in
//! safe Rust: no file descriptors, events or callbacks, which the adapter
//! in mod.rs keeps.
//!
//! A job finishes when both its process has exited and its output is
//! closed, in either order. Whichever comes second completes it.
#![forbid(unsafe_code)]

/// Identifies a job. Never reused, so a job that has gone finds nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(pub u32);

/// Where a job is between starting and finishing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JobState {
    /// The process runs and its output is open.
    #[default]
    Running,
    /// The process has exited, its output is still open.
    Dead,
    /// Its output is closed, the process still runs.
    Closed,
}

/// What the adapter does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStep {
    /// Run the completion callback and free the job.
    Complete,
    /// Wait for the other half.
    Wait,
}

/// The parts of a job that need no I/O.
#[derive(Clone, Debug)]
pub struct JobInfo {
    pub id: JobId,
    /// The command, as shown by show-messages -J.
    pub cmd: Vec<u8>,
    /// The process, -1 once it has exited.
    pub pid: i32,
    /// The wait status of the process once it exited.
    pub status: i32,
    /// The file descriptor of its output, -1 when there is none.
    pub fd: i32,
    pub state: JobState,
    /// The server does not wait for it at exit (`JOB_NOWAIT`).
    pub nowait: bool,
}

impl JobInfo {
    /// A new running job.
    pub fn new(id: JobId, cmd: Vec<u8>, pid: i32, nowait: bool) -> Self {
        Self {
            id,
            cmd,
            pid,
            status: 0,
            fd: -1,
            state: JobState::Running,
            nowait,
        }
    }

    /// The process exited with `status` (`job_check_died`, after stopped
    /// processes are handled).
    pub fn exited(&mut self, status: i32) -> JobStep {
        self.status = status;
        if self.state == JobState::Closed {
            return JobStep::Complete;
        }
        self.pid = -1;
        self.state = JobState::Dead;
        JobStep::Wait
    }

    /// The output closed or failed (`job_error_callback`).
    pub fn closed(&mut self) -> JobStep {
        if self.state == JobState::Dead {
            return JobStep::Complete;
        }
        self.state = JobState::Closed;
        JobStep::Wait
    }

    /// Whether the server waits for it before exiting (`job_still_running`).
    pub fn keeps_server(&self) -> bool {
        !self.nowait && self.state == JobState::Running
    }

    /// Its line in show-messages -J, as number `n`.
    pub fn summary(&self, n: usize) -> Vec<u8> {
        let mut out = format!("Job {n}: ").into_bytes();
        out.extend_from_slice(&self.cmd);
        out.extend_from_slice(
            format!(" [fd={}, pid={}, status={}]", self.fd, self.pid, self.status).as_bytes(),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> JobInfo {
        JobInfo::new(JobId(1), b"sleep 1".to_vec(), 42, false)
    }

    #[test]
    fn exit_then_close() {
        let mut j = job();
        assert_eq!(j.exited(256), JobStep::Wait);
        assert_eq!((j.state, j.pid, j.status), (JobState::Dead, -1, 256));
        assert!(!j.keeps_server());
        assert_eq!(j.closed(), JobStep::Complete);
    }

    #[test]
    fn close_then_exit() {
        let mut j = job();
        assert_eq!(j.closed(), JobStep::Wait);
        assert_eq!(j.state, JobState::Closed);
        assert_eq!(j.exited(0), JobStep::Complete);
        // The pid is kept when the exit completes the job, as in C.
        assert_eq!(j.pid, 42);
    }

    #[test]
    fn keeps_the_server_while_running_and_waited_for() {
        assert!(job().keeps_server());
        assert!(!JobInfo::new(JobId(2), Vec::new(), 1, true).keeps_server());
    }

    #[test]
    fn summary_line() {
        let mut j = job();
        j.fd = 7;
        assert_eq!(j.summary(0), b"Job 0: sleep 1 [fd=7, pid=42, status=0]");
    }
}
