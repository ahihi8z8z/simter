//! Answers OpenSSH password, passphrase and confirmation prompts for the
//! app's ssh processes (the terminal's and sftp's) with a dialog.
//!
//! ssh runs this app's own executable as `SSH_ASKPASS`. That helper process
//! forwards the prompt over a named pipe to the running app, which answers
//! from the tab's memory or asks the user, and prints the reply for ssh.
//!
//! Answers are kept per tab session, in memory, and dropped when the tab
//! closes. A remembered or saved answer is offered once per ssh process; if
//! ssh asks again, it was rejected: it is forgotten (and removed from the
//! [`Store`] if saved) and the user is asked instead. Passwords reach the
//! store only when the user ticks "Save password".

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::oneshot;

/// Set only in the environment of ssh processes started by the app; its
/// presence makes the executable act as the askpass helper.
pub const PIPE_ENV: &str = "SIMTER_ASKPASS_PIPE";
pub const TOKEN_ENV: &str = "SIMTER_ASKPASS_TOKEN";

/// Longest request accepted from a helper.
const MAX_REQUEST: u64 = 64 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    token: String,
    prompt: String,
    /// `SSH_ASKPASS_PROMPT`: `confirm`, `none`, or absent for a secret.
    hint: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Reply {
    answer: Option<String>,
}

/// Runs the helper when ssh started this executable as `SSH_ASKPASS`.
/// Returns its exit code, or `None` for a normal app start.
pub fn helper_main() -> Option<i32> {
    let pipe = std::env::var(PIPE_ENV).ok()?;
    let request = Request {
        token: std::env::var(TOKEN_ENV).unwrap_or_default(),
        prompt: std::env::args().nth(1).unwrap_or_default(),
        hint: std::env::var("SSH_ASKPASS_PROMPT").ok(),
    };
    Some(match ask_app(&pipe, &request) {
        Ok(Some(answer)) => {
            let mut out = io::stdout().lock();
            match writeln!(out, "{answer}").and_then(|()| out.flush()) {
                Ok(()) => 0,
                Err(_) => 1,
            }
        }
        // No answer: ssh treats a failed helper as a cancelled prompt.
        Ok(None) => 1,
        Err(e) => {
            eprintln!("simter askpass: {e}");
            1
        }
    })
}

fn ask_app(pipe: &str, request: &Request) -> io::Result<Option<String>> {
    let file = open_pipe(pipe)?;
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    (&file).write_all(line.as_bytes())?;
    let mut reply = String::new();
    BufReader::new(&file).read_line(&mut reply)?;
    Ok(serde_json::from_str::<Reply>(&reply)?.answer)
}

fn open_pipe(pipe: &str) -> io::Result<File> {
    const ERROR_PIPE_BUSY: i32 = 231;
    let mut attempts = 0;
    loop {
        match OpenOptions::new().read(true).write(true).open(pipe) {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempts < 50 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(100));
            }
            result => return result,
        }
    }
}

/// What kind of answer a prompt needs.
#[derive(Debug, PartialEq, Eq)]
enum Prompt {
    /// Informational (`SSH_ASKPASS_PROMPT=none`); ssh does not wait for it.
    Info,
    /// Yes/no; `yes` is what ssh expects for "yes".
    Confirm { yes: &'static str },
    /// A password or passphrase, remembered under `key`.
    Secret { key: String },
}

const BAD_PASSPHRASE: &str = "Bad passphrase, try again for ";

fn classify(prompt: &str, hint: Option<&str>) -> Prompt {
    match hint {
        Some("none") => Prompt::Info,
        // ssh reads the helper's exit status, not its output.
        Some("confirm") => Prompt::Confirm { yes: "" },
        _ if prompt.contains("(yes/no") => Prompt::Confirm { yes: "yes" },
        // ssh words the retry differently; file it under the first prompt.
        _ => match prompt
            .strip_prefix(BAD_PASSPHRASE)
            .and_then(|rest| rest.strip_suffix(": "))
        {
            Some(key_file) => Prompt::Secret {
                key: format!("Enter passphrase for key '{key_file}': "),
            },
            None => Prompt::Secret {
                key: prompt.to_string(),
            },
        },
    }
}

/// Sent to the frontend, which shows a dialog and calls [`Broker::reply`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub id: u32,
    pub alias: String,
    pub prompt: String,
    /// A password or passphrase, rather than a yes/no question.
    pub secret: bool,
    /// ssh rejected the previous answer to this prompt.
    pub retry: bool,
    /// A problem with saved passwords, shown with the question.
    pub error: Option<String>,
}

/// Where passwords the user chose to save are kept, by target name.
pub trait Store: Send + Sync {
    fn get(&self, target: &str) -> Result<Option<String>, String>;
    fn set(&self, target: &str, secret: &str) -> Result<(), String>;
    /// Succeeds when there is nothing to delete.
    fn delete(&self, target: &str) -> Result<(), String>;
}

/// Saved passwords are per host alias: a server-chosen prompt such as
/// "Password: " must not pick up another host's password.
fn target(alias: &str, key: &str) -> String {
    format!("simter:{alias}:{}", key.trim_end())
}

/// Where the app saved passwords before it was renamed to Simter.
fn legacy_target(alias: &str, key: &str) -> String {
    format!("ssh-workspace:{alias}:{}", key.trim_end())
}

struct Pending {
    sender: oneshot::Sender<Option<String>>,
    /// Store target for a password the user may choose to save.
    target: Option<String>,
}

/// One tab's remembered answers.
struct Session {
    alias: String,
    answers: Mutex<HashMap<String, String>>,
    /// One question at a time per tab; later prompts reuse the answer.
    turn: tokio::sync::Mutex<()>,
}

/// One ssh process: the prompts it has already been answered.
struct Process {
    session: Arc<Session>,
    asked: Mutex<HashSet<String>>,
    /// The user cancelled a prompt; ssh's retries are refused without asking.
    cancelled: AtomicBool,
}

/// Environment that routes one ssh process's prompts to the broker.
pub struct Askpass {
    program: PathBuf,
    pipe: String,
    token: String,
}

impl Askpass {
    pub fn apply(&self, cmd: &mut Command) {
        for (name, value) in self.env() {
            cmd.env(name, value);
        }
    }

    pub fn env(&self) -> [(&str, std::ffi::OsString); 4] {
        [
            ("SSH_ASKPASS", self.program.clone().into_os_string()),
            // Use the helper even though ssh has no display or console.
            ("SSH_ASKPASS_REQUIRE", "force".into()),
            (PIPE_ENV, self.pipe.clone().into()),
            (TOKEN_ENV, self.token.clone().into()),
        ]
    }
}

type Notify = Box<dyn Fn(Question) -> Result<(), String> + Send + Sync>;

pub struct Broker {
    program: PathBuf,
    pipe: String,
    notify: Notify,
    store: Box<dyn Store>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    processes: Mutex<HashMap<String, Arc<Process>>>,
    pending: Mutex<HashMap<u32, Pending>>,
    next_id: AtomicU32,
}

fn random_hex() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("OS random number generator");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Broker {
    /// `program` is the helper executable; `store` keeps saved passwords;
    /// `notify` delivers questions to the user.
    pub fn new(
        program: PathBuf,
        store: impl Store + 'static,
        notify: impl Fn(Question) -> Result<(), String> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Broker {
            program,
            pipe: format!(
                r"\\.\pipe\simter-askpass-{}-{}",
                std::process::id(),
                random_hex()
            ),
            notify: Box::new(notify),
            store: Box::new(store),
            sessions: Mutex::default(),
            processes: Mutex::default(),
            pending: Mutex::default(),
            next_id: AtomicU32::new(1),
        })
    }

    /// Creates the pipe and serves helpers in the background.
    pub async fn start(self: &Arc<Self>) -> io::Result<()> {
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&self.pipe)?;
        let broker = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                if let Err(e) = server.connect().await {
                    eprintln!("askpass pipe: {e}");
                    continue;
                }
                let client = server;
                server = match ServerOptions::new()
                    .reject_remote_clients(true)
                    .create(&broker.pipe)
                {
                    Ok(next) => next,
                    Err(e) => {
                        eprintln!("askpass pipe stopped: {e}");
                        return;
                    }
                };
                let broker = broker.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = broker.serve(client).await {
                        eprintln!("askpass request failed: {e}");
                    }
                });
            }
        });
        Ok(())
    }

    async fn serve(&self, pipe: NamedPipeServer) -> io::Result<()> {
        let (reader, mut writer) = tokio::io::split(pipe);
        let mut line = String::new();
        tokio::io::BufReader::new(reader.take(MAX_REQUEST))
            .read_line(&mut line)
            .await?;
        let request: Request = serde_json::from_str(&line)?;
        let answer = self.answer(request).await;
        let mut reply = serde_json::to_string(&Reply { answer })?;
        reply.push('\n');
        writer.write_all(reply.as_bytes()).await?;
        writer.flush().await
    }

    /// Environment for a new ssh process of tab `session` on `alias`.
    pub fn askpass(&self, session: &str, alias: &str) -> Askpass {
        let session = self
            .sessions
            .lock()
            .unwrap()
            .entry(session.to_string())
            .or_insert_with(|| {
                Arc::new(Session {
                    alias: alias.to_string(),
                    answers: Mutex::default(),
                    turn: tokio::sync::Mutex::new(()),
                })
            })
            .clone();
        let token = random_hex();
        self.processes.lock().unwrap().insert(
            token.clone(),
            Arc::new(Process {
                session,
                asked: Mutex::default(),
                cancelled: AtomicBool::new(false),
            }),
        );
        Askpass {
            program: self.program.clone(),
            pipe: self.pipe.clone(),
            token,
        }
    }

    /// Forgets a closed tab's answers. Its ssh processes get no more answers.
    pub fn end_session(&self, session: &str) {
        if let Some(ended) = self.sessions.lock().unwrap().remove(session) {
            self.processes
                .lock()
                .unwrap()
                .retain(|_, p| !Arc::ptr_eq(&p.session, &ended));
        }
    }

    /// Delivers the user's answer to question `id`; `None` means cancelled.
    /// With `save`, a password is also saved in the store first; ssh gets the
    /// answer even if saving fails, and the error is returned.
    pub fn reply(&self, id: u32, answer: Option<String>, save: bool) -> Result<(), String> {
        let Some(pending) = self.pending.lock().unwrap().remove(&id) else {
            return Ok(());
        };
        let saved = match (&answer, &pending.target) {
            (Some(secret), Some(target)) if save => self.store.set(target, secret),
            _ => Ok(()),
        };
        let _ = pending.sender.send(answer);
        saved
    }

    async fn answer(&self, request: Request) -> Option<String> {
        // Unknown tokens come from ended sessions or other programs.
        let process = self.processes.lock().unwrap().get(&request.token)?.clone();
        let session = &process.session;
        let _turn = session.turn.lock().await;
        if process.cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let answer = self.answer_in_turn(&process, request).await;
        if answer.is_none() {
            process.cancelled.store(true, Ordering::Relaxed);
        }
        answer
    }

    /// Reads a saved password, moving one saved under the old app name.
    fn saved(&self, target: &str, legacy: &str) -> Result<Option<String>, String> {
        if let Some(saved) = self.store.get(target)? {
            return Ok(Some(saved));
        }
        let Some(saved) = self.store.get(legacy)? else {
            return Ok(None);
        };
        self.store.set(target, &saved)?;
        self.store.delete(legacy)?;
        Ok(Some(saved))
    }

    async fn answer_in_turn(&self, process: &Process, request: Request) -> Option<String> {
        let session = &process.session;
        match classify(&request.prompt, request.hint.as_deref()) {
            Prompt::Info => Some(String::new()),
            Prompt::Confirm { yes } => self
                .ask(session, &request.prompt, None, false, None)
                .await
                .map(|_| yes.to_string()),
            Prompt::Secret { key } => {
                let target = target(&session.alias, &key);
                let first = process.asked.lock().unwrap().insert(key.clone());
                let error = if first {
                    if let Some(known) = session.answers.lock().unwrap().get(&key) {
                        return Some(known.clone());
                    }
                    match self.saved(&target, &legacy_target(&session.alias, &key)) {
                        Ok(Some(saved)) => return Some(saved),
                        Ok(None) => None,
                        Err(e) => Some(e),
                    }
                } else {
                    // Rejected, wherever it came from.
                    session.answers.lock().unwrap().remove(&key);
                    self.store.delete(&target).err()
                };
                let answer = self
                    .ask(session, &request.prompt, Some(target), !first, error)
                    .await?;
                session.answers.lock().unwrap().insert(key, answer.clone());
                Some(answer)
            }
        }
    }

    /// Asks the user. `target` is set for passwords, which may be saved.
    async fn ask(
        &self,
        session: &Session,
        prompt: &str,
        target: Option<String>,
        retry: bool,
        error: Option<String>,
    ) -> Option<String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        let secret = target.is_some();
        self.pending
            .lock()
            .unwrap()
            .insert(id, Pending { sender, target });
        // Time spent typing must not count against operation timeouts.
        let _pause = crate::process::pause_timeouts();
        let question = Question {
            id,
            alias: session.alias.clone(),
            prompt: prompt.to_string(),
            secret,
            retry,
            error,
        };
        if let Err(e) = (self.notify)(question) {
            eprintln!("cannot show the askpass prompt: {e}");
            self.pending.lock().unwrap().remove(&id);
            return None;
        }
        receiver.await.ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn classifies_prompts() {
        assert_eq!(
            classify("user@203.0.113.5's password: ", None),
            Prompt::Secret {
                key: "user@203.0.113.5's password: ".into()
            }
        );
        assert_eq!(
            classify(
                "Bad passphrase, try again for C:\\Users\\me\\.ssh\\id_rsa: ",
                None
            ),
            Prompt::Secret {
                key: "Enter passphrase for key 'C:\\Users\\me\\.ssh\\id_rsa': ".into()
            }
        );
        let host_key = "The authenticity of host 'x (1.2.3.4)' can't be established.\r\n\
            Are you sure you want to continue connecting (yes/no/[fingerprint])? ";
        assert_eq!(classify(host_key, None), Prompt::Confirm { yes: "yes" });
        assert_eq!(
            classify("Allow use of key?", Some("confirm")),
            Prompt::Confirm { yes: "" }
        );
        assert_eq!(classify("Touch your key", Some("none")), Prompt::Info);
    }

    /// In-memory [`Store`]; `fail` makes every call fail.
    #[derive(Clone, Default)]
    struct Saved {
        map: Arc<Mutex<HashMap<String, String>>>,
        fail: Arc<AtomicBool>,
    }

    impl Saved {
        fn check(&self) -> Result<(), String> {
            match self.fail.load(Ordering::Relaxed) {
                true => Err("store broken".into()),
                false => Ok(()),
            }
        }
    }

    impl Store for Saved {
        fn get(&self, target: &str) -> Result<Option<String>, String> {
            self.check()?;
            Ok(self.map.lock().unwrap().get(target).cloned())
        }
        fn set(&self, target: &str, secret: &str) -> Result<(), String> {
            self.check()?;
            self.map
                .lock()
                .unwrap()
                .insert(target.into(), secret.into());
            Ok(())
        }
        fn delete(&self, target: &str) -> Result<(), String> {
            self.check()?;
            self.map.lock().unwrap().remove(target);
            Ok(())
        }
    }

    struct Harness {
        broker: Arc<Broker>,
        questions: mpsc::Receiver<Question>,
        saved: Saved,
    }

    impl Harness {
        fn new() -> Self {
            let (sender, questions) = mpsc::channel();
            let sender = Mutex::new(sender);
            let saved = Saved::default();
            let broker = Broker::new(PathBuf::from("helper.exe"), saved.clone(), move |q| {
                sender.lock().unwrap().send(q).map_err(|e| e.to_string())
            });
            tauri::async_runtime::block_on(broker.start()).unwrap();
            Harness {
                broker,
                questions,
                saved,
            }
        }

        fn reply(&self, q: &Question, answer: Option<&str>) {
            self.broker
                .reply(q.id, answer.map(String::from), false)
                .unwrap();
        }

        /// Sends a prompt as the helper would, on a thread.
        fn prompt(
            &self,
            askpass: &Askpass,
            prompt: &str,
        ) -> std::thread::JoinHandle<io::Result<Option<String>>> {
            let pipe = askpass.pipe.clone();
            let request = Request {
                token: askpass.token.clone(),
                prompt: prompt.to_string(),
                hint: None,
            };
            std::thread::spawn(move || ask_app(&pipe, &request))
        }

        fn question(&self) -> Question {
            self.questions
                .recv_timeout(Duration::from_secs(5))
                .expect("a question for the user")
        }

        fn no_question(&self) {
            assert!(self
                .questions
                .recv_timeout(Duration::from_millis(200))
                .is_err());
        }
    }

    const PASSWORD: &str = "user@host's password: ";

    #[test]
    fn asks_once_per_session_and_again_when_rejected() {
        let h = Harness::new();
        let first = h.broker.askpass("tab1", "mec2");
        let reply = h.prompt(&first, PASSWORD);
        let q = h.question();
        assert_eq!((q.alias.as_str(), q.prompt.as_str()), ("mec2", PASSWORD));
        assert!(q.secret && !q.retry);
        h.reply(&q, Some("pw 1"));
        assert_eq!(reply.join().unwrap().unwrap().as_deref(), Some("pw 1"));

        // Another process of the same tab gets the remembered answer.
        let second = h.broker.askpass("tab1", "mec2");
        let reply = h.prompt(&second, PASSWORD);
        assert_eq!(reply.join().unwrap().unwrap().as_deref(), Some("pw 1"));
        h.no_question();

        // The same process asking again means the answer was rejected.
        let reply = h.prompt(&second, PASSWORD);
        let q = h.question();
        assert!(q.retry);
        h.reply(&q, Some("pw 2"));
        assert_eq!(reply.join().unwrap().unwrap().as_deref(), Some("pw 2"));

        // Other tabs do not share answers.
        let other = h.broker.askpass("tab2", "mec2");
        let reply = h.prompt(&other, PASSWORD);
        let q = h.question();
        h.reply(&q, None);
        assert_eq!(reply.join().unwrap().unwrap(), None);

        // ssh retries after a cancel; the user is not asked again.
        assert_eq!(h.prompt(&other, PASSWORD).join().unwrap().unwrap(), None);
        h.no_question();
    }

    #[test]
    fn host_keys_need_the_user() {
        let h = Harness::new();
        let askpass = h.broker.askpass("tab", "new-host");
        let prompt = "Are you sure you want to continue connecting (yes/no/[fingerprint])? ";
        let reply = h.prompt(&askpass, prompt);
        let q = h.question();
        assert!(!q.secret);
        h.reply(&q, Some(""));
        assert_eq!(reply.join().unwrap().unwrap().as_deref(), Some("yes"));

        // Never remembered: a second process asks again.
        let reply = h.prompt(&h.broker.askpass("tab", "new-host"), prompt);
        let q = h.question();
        h.reply(&q, None);
        assert_eq!(reply.join().unwrap().unwrap(), None);
    }

    #[test]
    fn ended_sessions_and_unknown_tokens_get_no_answer() {
        let h = Harness::new();
        let askpass = h.broker.askpass("tab", "mec2");
        let reply = h.prompt(&askpass, PASSWORD);
        let q = h.question();
        h.reply(&q, Some("pw"));
        reply.join().unwrap().unwrap();

        h.broker.end_session("tab");
        assert_eq!(h.prompt(&askpass, PASSWORD).join().unwrap().unwrap(), None);
        let fresh = h.broker.askpass("tab", "mec2");
        let forged = Askpass {
            token: "0".repeat(32),
            ..fresh
        };
        assert_eq!(h.prompt(&forged, PASSWORD).join().unwrap().unwrap(), None);
        h.no_question();
    }

    #[test]
    fn prompts_of_one_tab_are_asked_one_at_a_time() {
        let h = Harness::new();
        let a = h.prompt(&h.broker.askpass("tab", "mec2"), PASSWORD);
        let b = h.prompt(&h.broker.askpass("tab", "mec2"), PASSWORD);
        let q = h.question();
        h.no_question();
        h.reply(&q, Some("pw"));
        assert_eq!(a.join().unwrap().unwrap().as_deref(), Some("pw"));
        assert_eq!(b.join().unwrap().unwrap().as_deref(), Some("pw"));
    }

    #[test]
    fn saved_passwords_are_used_and_removed_when_rejected() {
        let h = Harness::new();
        let first = h.broker.askpass("tab1", "mec2");
        let reply = h.prompt(&first, PASSWORD);
        let q = h.question();
        h.broker.reply(q.id, Some("pw".into()), true).unwrap();
        assert_eq!(reply.join().unwrap().unwrap().as_deref(), Some("pw"));
        let target = "simter:mec2:user@host's password:";
        assert_eq!(h.saved.map.lock().unwrap()[target], "pw");

        // A new tab uses the saved password without asking.
        let other = h.broker.askpass("tab2", "mec2");
        assert_eq!(
            h.prompt(&other, PASSWORD)
                .join()
                .unwrap()
                .unwrap()
                .as_deref(),
            Some("pw")
        );
        h.no_question();
        // Another alias does not.
        let reply = h.prompt(&h.broker.askpass("tab3", "other"), PASSWORD);
        h.reply(&h.question(), None);
        assert_eq!(reply.join().unwrap().unwrap(), None);

        // Rejected: removed from the store, and the user is asked.
        let reply = h.prompt(&other, PASSWORD);
        let q = h.question();
        assert!(q.retry && q.error.is_none());
        assert!(h.saved.map.lock().unwrap().is_empty());
        h.reply(&q, Some("new"));
        assert_eq!(reply.join().unwrap().unwrap().as_deref(), Some("new"));
        assert!(h.saved.map.lock().unwrap().is_empty());
    }

    #[test]
    fn passwords_saved_under_the_old_name_are_moved() {
        let h = Harness::new();
        let legacy = "ssh-workspace:mec2:user@host's password:";
        h.saved
            .map
            .lock()
            .unwrap()
            .insert(legacy.into(), "pw".into());
        let reply = h.prompt(&h.broker.askpass("tab", "mec2"), PASSWORD);
        assert_eq!(reply.join().unwrap().unwrap().as_deref(), Some("pw"));
        h.no_question();
        let map = h.saved.map.lock().unwrap();
        assert_eq!(map.get(legacy), None);
        assert_eq!(map["simter:mec2:user@host's password:"], "pw");
    }

    #[test]
    fn store_failures_are_reported_not_hidden() {
        let h = Harness::new();
        h.saved.fail.store(true, Ordering::Relaxed);
        let reply = h.prompt(&h.broker.askpass("tab", "mec2"), PASSWORD);
        let q = h.question();
        assert_eq!(q.error.as_deref(), Some("store broken"));
        // ssh still gets the answer when saving fails.
        let saved = h.broker.reply(q.id, Some("pw".into()), true);
        assert_eq!(saved, Err("store broken".into()));
        assert_eq!(reply.join().unwrap().unwrap().as_deref(), Some("pw"));
    }

    /// Runs the real helper executable through sftp. Needs `cargo build`
    /// first, and `SIMTER_TEST_ALIAS` naming a host that logs in with a key.
    /// An empty known_hosts makes ssh ask to accept the host key.
    #[test]
    #[ignore = "needs a real SSH host"]
    fn real_host_key_prompt_through_helper() {
        let alias = std::env::var("SIMTER_TEST_ALIAS").expect("SIMTER_TEST_ALIAS");
        let helper = std::env::current_exe()
            .unwrap()
            .parent()
            .and_then(|deps| deps.parent())
            .unwrap()
            .join("simter.exe");
        assert!(
            helper.exists(),
            "run cargo build first: {}",
            helper.display()
        );
        let dir = tempfile::tempdir().unwrap();
        let home = std::env::var("USERPROFILE").unwrap();
        let config = dir.path().join("config");
        let tmp = dir.path().display().to_string().replace('\\', "/");
        let home = home.replace('\\', "/");
        std::fs::write(
            &config,
            format!(
                "Host *\n  UserKnownHostsFile \"{tmp}/known_hosts\"\n  \
                 GlobalKnownHostsFile \"{tmp}/none\"\n  StrictHostKeyChecking ask\n  \
                 Include \"{home}/.ssh/config\"\n"
            ),
        )
        .unwrap();

        let asked = Arc::new(Mutex::new(Vec::new()));
        let weak = Arc::new(Mutex::new(std::sync::Weak::<Broker>::new()));
        let (log, slot) = (asked.clone(), weak.clone());
        let broker = Broker::new(helper, Saved::default(), move |q| {
            log.lock().unwrap().push(q.prompt.clone());
            let broker = slot.lock().unwrap().upgrade().ok_or("broker gone")?;
            // Accept host keys, refuse anything else.
            std::thread::spawn(move || broker.reply(q.id, (!q.secret).then(String::new), false));
            Ok(())
        });
        *weak.lock().unwrap() = Arc::downgrade(&broker);
        tauri::async_runtime::block_on(broker.start()).unwrap();

        let sftp = crate::ssh_bin::locate("sftp.exe").unwrap();
        let connect = || {
            let askpass = broker.askpass("tab", &alias);
            crate::sftp::sftp_command(&sftp, Some(&config), &alias, Some(&askpass))
        };
        let listing = crate::sftp::list(&connect, None, Duration::from_secs(60))
            .unwrap_or_else(|e| panic!("{e:?}; prompts: {:?}", asked.lock().unwrap()));
        assert!(listing.path.starts_with('/'));
        let asked = asked.lock().unwrap();
        assert!(
            asked.iter().any(|p| p.contains("(yes/no")),
            "expected a host key prompt, got {asked:?}"
        );
        let known = std::fs::read_to_string(dir.path().join("known_hosts")).unwrap();
        assert!(!known.trim().is_empty());
    }
}
