//! One side of the comparison: a tmux binary and its server.
//!
//! Every command runs with a cleared environment and a fixed set of
//! variables, `-f/dev/null`, `-u` (UTF-8 output whatever the locale) and
//! `-S` with a socket in the run's own directory. The server is started
//! by the first setup command and watched through /proc, so a crash shows
//! up even when the client that triggered it exits normally.

use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How a command ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Exited with this code.
    Exited(i32),
    /// Killed by a signal (the client crashed).
    Signalled,
    /// Did not finish in time and was killed.
    Timeout,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Status::Exited(n) => write!(f, "exit {n}"),
            Status::Signalled => write!(f, "client killed by a signal"),
            Status::Timeout => write!(f, "timeout"),
        }
    }
}

/// What a command printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// How it ended.
    pub status: Status,
    /// Its standard output.
    pub stdout: Vec<u8>,
    /// Its standard error.
    pub stderr: Vec<u8>,
}

/// A tmux binary and the server it runs.
pub struct Side {
    /// A short name for reports ("tmux-rs", "tmux 3.8").
    pub name: String,
    /// The binary.
    pub bin: PathBuf,
    /// The socket path.
    socket: PathBuf,
    /// The directory the server is started in. tmux-rs writes its panic
    /// files there.
    workdir: PathBuf,
    /// The environment every command gets.
    env: Vec<(String, String)>,
    /// The server's pid, once it is set up.
    server_pid: Option<u32>,
}

impl Side {
    /// Creates a side. Nothing runs yet.
    pub fn new(
        name: &str,
        bin: PathBuf,
        socket: PathBuf,
        workdir: PathBuf,
        env: Vec<(String, String)>,
    ) -> Self {
        Side {
            name: name.to_string(),
            bin,
            socket,
            workdir,
            env,
            server_pid: None,
        }
    }

    /// Builds a command for this side with its arguments.
    fn command<S: AsRef<OsStr>>(&self, args: &[S]) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.env_clear()
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .current_dir(&self.workdir)
            .arg("-S")
            .arg(&self.socket)
            .arg("-f/dev/null")
            .arg("-u")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    /// Runs a command and waits for it at most `timeout`.
    pub fn run<S: AsRef<OsStr>>(&self, args: &[S], timeout: Duration) -> Outcome {
        let mut child = match self.command(args).spawn() {
            Ok(c) => c,
            Err(e) => {
                return Outcome {
                    status: Status::Exited(-1),
                    stdout: Vec::new(),
                    stderr: format!("cannot run {}: {e}", self.bin.display()).into_bytes(),
                };
            }
        };
        let out = child.stdout.take().map(reader);
        let err = child.stderr.take().map(reader);
        let start = Instant::now();
        let mut pause = Duration::from_micros(100);
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break status_of(s),
                Ok(None) if start.elapsed() >= timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Status::Timeout;
                }
                Ok(None) => {
                    thread::sleep(pause);
                    pause = (pause * 2).min(Duration::from_millis(5));
                }
                Err(_) => break Status::Signalled,
            }
        };
        // The client that starts a server forks it, and the server could
        // keep the pipes open. So the readers get a little time after the
        // client is gone and are abandoned after that.
        let stdout = out.map(collect).unwrap_or_default();
        let stderr = err.map(collect).unwrap_or_default();
        Outcome {
            status,
            stdout,
            stderr,
        }
    }

    /// Starts the server and runs the setup commands. A server already on
    /// the socket is killed first.
    pub fn start(&mut self, setup: &[Vec<String>], timeout: Duration) -> Result<(), String> {
        self.stop();
        let _ = fs::remove_file(&self.socket);
        for args in setup {
            let o = self.run(args, timeout);
            if o.status != Status::Exited(0) {
                return Err(format!(
                    "{}: setup command {:?} failed ({}): {}",
                    self.name,
                    args,
                    o.status,
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
        }
        let o = self.run(&["display-message", "-p", "#{pid}"], timeout);
        let pid = String::from_utf8_lossy(&o.stdout).trim().parse::<u32>();
        match pid {
            Ok(p) => {
                self.server_pid = Some(p);
                Ok(())
            }
            Err(_) => Err(format!("{}: cannot read the server pid", self.name)),
        }
    }

    /// Returns true while the server process exists and is not a zombie.
    pub fn alive(&self) -> bool {
        let Some(pid) = self.server_pid else {
            return false;
        };
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            // The state is the field after the command name, which is in
            // parentheses and may contain spaces.
            Ok(stat) => match stat.rfind(')') {
                Some(i) => !stat[i + 1..].trim_start().starts_with('Z'),
                None => true,
            },
            Err(_) => false,
        }
    }

    /// Stops the server: kill-server, then SIGKILL if it is still there.
    pub fn stop(&mut self) {
        if self.socket.exists() {
            let _ = self.run(&["kill-server"], Duration::from_secs(2));
        }
        if let Some(pid) = self.server_pid {
            let deadline = Instant::now() + Duration::from_secs(2);
            while self.alive() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if self.alive() {
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
            }
        }
        self.server_pid = None;
        let _ = fs::remove_file(&self.socket);
    }

    /// Takes the panic files the server left in its directory (tmux-rs
    /// writes server-panic-PID.txt), returns their text and removes them.
    pub fn take_panic_files(&self) -> Vec<String> {
        take_panic_files(&self.workdir)
    }
}

impl Drop for Side {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Reads a pipe to its end on a thread, which sends the bytes back.
fn reader<R: Read + Send + 'static>(mut r: R) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx
}

/// Waits a second at most for what a [`reader`] read.
fn collect(rx: mpsc::Receiver<Vec<u8>>) -> Vec<u8> {
    rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default()
}

/// Turns a process status into a [`Status`].
fn status_of(s: ExitStatus) -> Status {
    match s.code() {
        Some(n) => Status::Exited(n),
        None => Status::Signalled,
    }
}

/// Reads and removes the server-panic files in a directory.
fn take_panic_files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("server-panic-") && name.ends_with(".txt") {
            if let Ok(text) = fs::read_to_string(e.path()) {
                out.push(text);
            }
            let _ = fs::remove_file(e.path());
        }
    }
    out
}
