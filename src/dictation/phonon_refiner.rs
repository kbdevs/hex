//! A final-only client for Phonon's separately installed text refiner.
//! Only the correction sidecar runs: Hex keeps its own speech model. Runtime
//! and weights must already be cached; dictation never downloads them.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, TryLockError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use color_eyre::eyre::{Result, bail, eyre};
use serde_json::{Value, json};

use super::context::ContextSnapshot;
use super::vocabulary::VocabularyEntry;

const MODEL: &str = "Infatoshi/phonon-refiner-1.7b";
const REVISION: &str = "2b0ab74bee7ab7cf364695bbe44f64ce4c47439e";
const MLX_REQUIREMENT: &str = "mlx-lm==0.31.3";
const POLL: Duration = Duration::from_millis(20);
const MAX_LINE: usize = 1024 * 1024;
const MAX_INPUT: usize = 128 * 1024;
static CLIENT: OnceLock<Mutex<Option<Client>>> = OnceLock::new();
static EPOCH: AtomicU64 = AtomicU64::new(0);

struct Bundle {
    contents: PathBuf,
}

impl Bundle {
    fn at(contents: PathBuf) -> Option<Self> {
        [
            "Helpers/uv",
            "Resources/sidecar/polish_server.py",
            "Resources/sidecar/linereader.py",
            "Resources/sidecar/models/lfm3.py",
            "Resources/prompts/polish_v2.txt",
        ]
        .iter()
        .all(|file| contents.join(file).is_file())
        .then_some(Self { contents })
    }

    fn installed() -> Result<Self> {
        let paths = std::iter::once(PathBuf::from("/Applications/Phonon.app/Contents")).chain(
            crate::app_paths::home_dir().map(|home| home.join("Applications/Phonon.app/Contents")),
        );
        paths.filter_map(Self::at).next().ok_or_else(|| {
            eyre!("Install Phonon and prepare its refiner first; see PHONON_REFINER.md")
        })
    }

    fn command(&self) -> Command {
        let resources = self.contents.join("Resources");
        let mut command =
            crate::desktop::background_process::command(self.contents.join("Helpers/uv"));
        command
            .args([
                "run",
                "--offline",
                "--no-project",
                "--python",
                "3.12",
                "--with",
                MLX_REQUIREMENT,
                "python",
            ])
            .arg(resources.join("sidecar/polish_server.py"))
            .args(["--model", MODEL, "--revision", REVISION])
            .arg("--system-prompt-file")
            .arg(resources.join("prompts/polish_v2.txt"))
            .current_dir(&resources)
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("PHONON_STREAM_REFINE", "0");
        let engine = self.contents.join("Helpers/pe");
        let shaders = resources.join("engine");
        if engine.is_file()
            && shaders.join("mk.metal").is_file()
            && shaders.join("df.metal").is_file()
        {
            command
                .arg("--engine-binary")
                .arg(engine)
                .arg("--engine-shaders")
                .arg(shaders);
        }
        command
    }
}

struct Client {
    child: Child,
    stdin: ChildStdin,
    responses: mpsc::Receiver<Result<Value, &'static str>>,
    ready: bool,
    reply_complete: bool,
}

impl Client {
    fn spawn(mut command: Command) -> Result<Self> {
        // A dedicated process group lets cancellation also stop uv's Python
        // child and the native Metal helper it starts.
        let mut child = command
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (sender, responses) = mpsc::sync_channel(8);
        let client = Self {
            child,
            stdin,
            responses,
            ready: false,
            reply_complete: false,
        };
        // A helper that stops reading must not trap a processing worker inside
        // write_all. Every pipe write below shares the request deadline.
        let fd = client.stdin.as_raw_fd();
        // SAFETY: fd belongs to the live child stdin; fcntl only changes its flags.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        thread::Builder::new()
            .name("phonon-replies".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let mut line = Vec::new();
                    match Read::by_ref(&mut reader)
                        .take((MAX_LINE + 1) as u64)
                        .read_until(b'\n', &mut line)
                    {
                        Ok(0) => break,
                        Ok(_) if line.len() <= MAX_LINE && line.ends_with(b"\n") => {
                            let value =
                                serde_json::from_slice(&line).map_err(|_| "invalid refiner reply");
                            let failed = value.is_err();
                            if sender.send(value).is_err() || failed {
                                break;
                            }
                        }
                        _ => {
                            let _ = sender.send(Err("refiner reply exceeds the limit"));
                            break;
                        }
                    }
                }
            })?;
        Ok(client)
    }

    fn send(&mut self, value: &Value, until: Instant, cancelled: &impl Fn() -> bool) -> Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        let mut remaining = bytes.as_slice();
        while !remaining.is_empty() {
            check(until, cancelled)?;
            match self.stdin.write(remaining) {
                Ok(0) => bail!("Phonon refiner input closed"),
                Ok(count) => remaining = &remaining[count..],
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(POLL),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn receive(&self, until: Instant, cancelled: &impl Fn() -> bool) -> Result<Value> {
        loop {
            check(until, cancelled)?;
            match self.responses.recv_timeout(POLL) {
                Ok(Ok(value)) if value.get("ok") == Some(&Value::Bool(true)) => return Ok(value),
                // Helper messages can contain its prompt. Keep them out of Hex's logs.
                Ok(_) => bail!("Phonon refiner returned an error or invalid reply"),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => bail!("Phonon refiner stopped"),
            }
        }
    }

    fn run(
        &mut self,
        payload: &Value,
        source: &str,
        until: Instant,
        cancelled: &impl Fn() -> bool,
    ) -> Result<String> {
        self.reply_complete = false;
        if !self.ready {
            self.send(&json!({"command": "warmup"}), until, cancelled)?;
            loop {
                let value = self.receive(until, cancelled)?;
                match value.pointer("/status/state").and_then(Value::as_str) {
                    Some("ready") => {
                        self.ready = true;
                        break;
                    }
                    Some("loading") => {}
                    _ => bail!("Phonon refiner did not finish setup"),
                }
            }
        }
        self.send(payload, until, cancelled)?;
        let value = self.receive(until, cancelled)?;
        if value.get("stream").is_some() || value.get("status").is_some() {
            bail!("Phonon refiner did not return final text");
        }
        self.reply_complete = value.get("outputText").is_some_and(Value::is_string);
        if value
            .pointer("/diagnostics/guardFired")
            .and_then(Value::as_bool)
            == Some(true)
        {
            bail!("Phonon refiner rejected its output");
        }
        safe_output(source, value.get("outputText").and_then(Value::as_str))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: process_group(0) gave only this child's descendants this
        // process group. The unreaped child keeps its PID from being reused.
        unsafe {
            libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

fn check(until: Instant, cancelled: &impl Fn() -> bool) -> Result<()> {
    if cancelled() || crate::desktop::process_exit::quitting() {
        bail!("Phonon refiner cancelled");
    }
    if Instant::now() >= until {
        bail!("Phonon refiner timed out");
    }
    Ok(())
}

/// Best-effort local rewrite, with one warm text model shared by processing
/// workers. Errors preserve the corrected transcript in the caller.
pub fn refine(
    text: &str,
    context: &ContextSnapshot,
    vocabulary: &[VocabularyEntry],
    deadline: Duration,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<String> {
    let until = Instant::now() + deadline.min(Duration::from_secs(30));
    let epoch = EPOCH.load(Ordering::Acquire);
    let cancelled = || {
        cancelled.load(Ordering::Acquire)
            || !crate::labs::phonon_refiner()
            || EPOCH.load(Ordering::Acquire) != epoch
    };
    check(until, &cancelled)?;
    let payload = request(text, context, vocabulary)?;
    let lock = CLIENT.get_or_init(|| Mutex::new(None));
    let mut guard = loop {
        check(until, &cancelled)?;
        match lock.try_lock() {
            Ok(guard) => break guard,
            Err(TryLockError::WouldBlock) => thread::sleep(POLL),
            Err(TryLockError::Poisoned(_)) => bail!("Phonon refiner worker failed"),
        }
    };
    if guard.is_none() {
        *guard = Some(Client::spawn(Bundle::installed()?.command())?);
    }
    let client = guard.as_mut().expect("initialized client");
    let result = client.run(&payload, text, until, &cancelled);
    if (result.is_err() && !client.reply_complete)
        || cancelled()
        || crate::desktop::process_exit::quitting()
    {
        *guard = None;
    }
    result
}

/// Turning the option off, or quitting, stops both the sidecar and its native
/// engine. A busy request observes the epoch within POLL and drops its client.
pub fn stop() {
    EPOCH.fetch_add(1, Ordering::AcqRel);
    if let Some(lock) = CLIENT.get() {
        match lock.try_lock() {
            Ok(mut guard) => *guard = None,
            Err(TryLockError::Poisoned(error)) => *error.into_inner() = None,
            Err(TryLockError::WouldBlock) => {}
        }
    }
}

fn request(text: &str, context: &ContextSnapshot, vocabulary: &[VocabularyEntry]) -> Result<Value> {
    if text.is_empty()
        || text.len() > MAX_INPUT
        || text.contains("<transcript>")
        || text.contains("</transcript>")
        || text.contains("<phonon_dictionary>")
    {
        bail!("Transcript is outside the local refiner input contract");
    }
    let terms = vocabulary
        .iter()
        .take(80)
        .map(|entry| {
            entry
                .term
                .chars()
                .take(128)
                .collect::<String>()
                .replace(['\n', '\r', ',', '<', '>'], " ")
        })
        .collect::<Vec<_>>()
        .join(", ");
    Ok(
        json!({"command": "run", "inputText": format!("<phonon_dictionary>\ncanonical_terms: {terms}\n</phonon_dictionary>\n<transcript>\n{text}\n</transcript>"), "context": {"app_category": category(context)}}),
    )
}

fn category(context: &ContextSnapshot) -> &'static str {
    match context
        .application
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "discord" | "messages" | "whatsapp" | "telegram" | "signal" => "chat",
        "slack" | "microsoft teams" => "work_chat",
        "mail" | "microsoft outlook" => "email",
        "terminal" | "iterm2" | "warp" | "wezterm" | "cursor" | "visual studio code" | "zed"
        | "opencode" => "code",
        "chatgpt" | "claude" => "ai_prompt",
        _ => match context
            .browser_url
            .as_ref()
            .and_then(|url| url.host_str())
            .unwrap_or_default()
        {
            "discord.com" | "web.whatsapp.com" | "web.telegram.org" => "chat",
            "app.slack.com" | "teams.microsoft.com" | "chat.google.com" => "work_chat",
            "mail.google.com" | "outlook.live.com" | "outlook.office.com" => "email",
            "chatgpt.com" | "claude.ai" => "ai_prompt",
            _ => "other",
        },
    }
}

fn safe_output(source: &str, candidate: Option<&str>) -> Result<String> {
    let candidate = candidate.unwrap_or_default().trim();
    if candidate.is_empty() {
        bail!("Phonon refiner returned empty text");
    }
    let source_words = source.split_whitespace().count();
    let output_words = candidate.split_whitespace().count();
    if output_words > source_words.saturating_mul(2)
        || (source_words > 8 && output_words.saturating_mul(10) < source_words.saturating_mul(4))
    {
        bail!("Phonon refiner output failed the length guard");
    }
    Ok(candidate.to_owned())
}

#[cfg(test)]
mod tests;
