//! Tauri commands exposed to the frontend. Blocking work runs off the UI thread.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Manager, State};

use crate::askpass::Broker;
use crate::editor;
use crate::hosts::{self, Host, Issue};
use crate::prefs::{self, Layout, Prefs};
use crate::resolve::{self, Details};
use crate::sftp::{self, Listing};
use crate::sftp_session::{self, Connection, Connections, Failure};
use crate::ssh_bin;
use crate::terminal::{self, PtySession, Terminals};
use crate::transfer::{self, Transfers};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostList {
    config_path: PathBuf,
    /// `%USERPROFILE%\.ssh\config`, whichever file was read.
    default_config_path: PathBuf,
    hosts: Vec<Host>,
    issues: Vec<Issue>,
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .ok_or_else(|| "USERPROFILE is not set; cannot locate the SSH config".to_string())
}

/// Opens `file` in an external editor, at `line` when the editor supports it.
/// The app itself never writes the config.
#[tauri::command]
pub fn open_config(app: AppHandle, file: PathBuf, line: Option<usize>) -> Result<(), String> {
    let prefs = prefs::load(&prefs_path(&app)?)?;
    editor::open(&file, line, prefs.editor.as_deref())
}

fn prefs_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join("prefs.json"))
        .map_err(|e| format!("cannot locate the app settings folder: {e}"))
}

#[tauri::command]
pub fn prefs_load(app: AppHandle) -> Result<Prefs, String> {
    let path = prefs_path(&app)?;
    // Settings saved before the app was renamed to Simter.
    if let Some(dir) = path.parent().and_then(Path::parent) {
        prefs::adopt(&dir.join(prefs::LEGACY_DIR).join("prefs.json"), &path)?;
    }
    prefs::load(&path)
}

#[tauri::command]
pub fn prefs_set_config_path(app: AppHandle, config_path: Option<PathBuf>) -> Result<(), String> {
    prefs::update(&prefs_path(&app)?, |p| p.config_path = config_path)
}

#[tauri::command]
pub fn prefs_set_layout(app: AppHandle, layout: Layout) -> Result<(), String> {
    prefs::update(&prefs_path(&app)?, |p| p.layout = layout)
}

/// Records that `alias` was opened; returns the recent hosts, most recent first.
#[tauri::command]
pub fn prefs_record_host(app: AppHandle, alias: String) -> Result<Vec<String>, String> {
    let mut recent = Vec::new();
    prefs::update(&prefs_path(&app)?, |p| {
        p.record_host(&alias);
        recent = p.recent_hosts.clone();
    })?;
    Ok(recent)
}

/// Lists host aliases from `config_path`, or `%USERPROFILE%\.ssh\config`.
#[tauri::command]
pub async fn list_hosts(config_path: Option<PathBuf>) -> Result<HostList, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let home = home_dir()?;
        let default_config_path = home.join(".ssh").join("config");
        let config_path = config_path.unwrap_or_else(|| default_config_path.clone());
        let env = |name: &str| std::env::var(name).ok();
        let found = hosts::discover(
            &config_path,
            &hosts::Context {
                home: &home,
                env: &env,
            },
        );
        Ok(HostList {
            config_path,
            default_config_path,
            hosts: found.hosts,
            issues: found.issues,
        })
    })
    .await
    .map_err(|e| format!("host discovery task failed: {e}"))?
}

/// Effective user/hostname/port for `alias`, as `ssh -G` resolves them.
#[tauri::command]
pub async fn host_details(alias: String, config_path: Option<PathBuf>) -> Result<Details, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let ssh = ssh_bin::locate("ssh.exe")?;
        resolve::resolve(
            &ssh,
            config_path.as_deref(),
            &alias,
            std::time::Duration::from_secs(5),
        )
    })
    .await
    .map_err(|e| format!("host details task failed: {e}"))?
}

/// The tab's long-lived SFTP connection to `alias`, opened on first use.
/// Prompts are answered within tab `session`.
async fn sftp_connection(
    broker: &Broker,
    connections: &Connections,
    session: &str,
    alias: &str,
    config_path: Option<&Path>,
) -> Result<Arc<Connection>, sftp::Error> {
    let ssh = ssh_bin::locate("ssh.exe")
        .map_err(|m| sftp::Error::new(sftp::ErrorKind::Unavailable, m))?;
    connections
        .get(session, alias, config_path, || {
            let mut cmd = tokio::process::Command::new(ssh);
            cmd.args(sftp_session::ssh_args(config_path, alias));
            for (name, value) in broker.askpass(session, alias).env() {
                cmd.env(name, value);
            }
            cmd
        })
        .await
}

/// Lists `path` on `alias` over the tab's SFTP connection, or the login
/// directory when `path` is absent.
#[tauri::command]
pub async fn sftp_list(
    broker: State<'_, Arc<Broker>>,
    connections: State<'_, Connections>,
    session: String,
    alias: String,
    path: Option<String>,
    config_path: Option<PathBuf>,
) -> Result<Listing, sftp::Error> {
    let connection = sftp_connection(
        &broker,
        &connections,
        &session,
        &alias,
        config_path.as_deref(),
    )
    .await?;
    let result = connection.list(path.as_deref()).await;
    settle(&connections, &session, &connection, result).await
}

/// Drops the connection after a failure that left it unusable, so the next
/// request reconnects.
async fn settle<T>(
    connections: &Connections,
    session: &str,
    connection: &Arc<Connection>,
    result: Result<T, Failure>,
) -> Result<T, sftp::Error> {
    match result {
        Ok(value) => Ok(value),
        Err(failure) => {
            if failure.lost {
                connections.discard(session, connection).await;
            }
            Err(failure.error)
        }
    }
}

/// Closes the tab's SFTP connection.
#[tauri::command]
pub fn sftp_close(connections: State<'_, Connections>, session: String) {
    connections.close(&session);
}

/// Runs one transfer off the UI thread with a cancel flag registered under
/// `id`, so [`transfer_cancel`] can stop it.
async fn run_transfer(
    app: AppHandle,
    id: u32,
    session: String,
    alias: String,
    config_path: Option<PathBuf>,
    work: impl FnOnce(sftp::Connect, &std::sync::atomic::AtomicBool) -> Result<(), sftp::Error>
        + Send
        + 'static,
) -> Result<(), sftp::Error> {
    tauri::async_runtime::spawn_blocking(move || {
        let sftp = ssh_bin::locate("sftp.exe")
            .map_err(|m| sftp::Error::new(sftp::ErrorKind::Unavailable, m))?;
        let broker = app.state::<Arc<Broker>>();
        let connect = || {
            let askpass = broker.askpass(&session, &alias);
            sftp::sftp_command(&sftp, config_path.as_deref(), &alias, Some(&askpass))
        };
        let transfers = app.state::<Transfers>();
        let cancel = transfers.begin(id);
        let result = work(&connect, &cancel);
        transfers.end(id);
        result
    })
    .await
    .map_err(|e| sftp::Error::new(sftp::ErrorKind::Other, format!("transfer task failed: {e}")))?
}

/// Uploads the local file or folder `local` into `remote_dir` on `alias`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sftp_upload(
    app: AppHandle,
    id: u32,
    session: String,
    alias: String,
    local: PathBuf,
    remote_dir: String,
    overwrite: bool,
    config_path: Option<PathBuf>,
) -> Result<(), sftp::Error> {
    run_transfer(
        app,
        id,
        session,
        alias,
        config_path,
        move |connect, cancel| transfer::upload(connect, &local, &remote_dir, overwrite, cancel),
    )
    .await
}

/// Downloads `name` from `remote_dir` on `alias` into `local_dir`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sftp_download(
    app: AppHandle,
    id: u32,
    session: String,
    alias: String,
    remote_dir: String,
    name: String,
    local_dir: PathBuf,
    overwrite: bool,
    config_path: Option<PathBuf>,
) -> Result<(), sftp::Error> {
    run_transfer(
        app,
        id,
        session,
        alias,
        config_path,
        move |connect, cancel| {
            // The process id keeps temporary names apart from earlier runs.
            let tag = format!("{}-{id}", std::process::id());
            transfer::download(
                connect,
                &remote_dir,
                &name,
                &local_dir,
                overwrite,
                &tag,
                cancel,
            )
        },
    )
    .await
}

#[tauri::command]
pub fn transfer_cancel(transfers: State<'_, Transfers>, id: u32) {
    transfers.cancel(id);
}

/// The user's answer to askpass question `id`; `None` means cancelled.
/// `save` also stores a password in Windows Credential Manager.
#[tauri::command]
pub fn askpass_reply(
    broker: State<'_, Arc<Broker>>,
    id: u32,
    answer: Option<String>,
    save: bool,
) -> Result<(), String> {
    broker.reply(id, answer, save)
}

/// Forgets the passwords remembered for a closed tab.
#[tauri::command]
pub fn askpass_end(broker: State<'_, Arc<Broker>>, session: String) {
    broker.end_session(&session);
}

/// Starts `ssh <alias>` in a pseudoconsole. `events` receives raw output
/// bytes, then one JSON [`Exit`] once the process has ended. One channel keeps
/// them in order, so the exit never overtakes the last output.
#[tauri::command]
pub async fn terminal_open(
    app: AppHandle,
    session: String,
    alias: String,
    config_path: Option<PathBuf>,
    cols: u16,
    rows: u16,
    events: Channel<InvokeResponseBody>,
) -> Result<u32, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let ssh = ssh_bin::locate("ssh.exe")?;
        let mut cmd = terminal::ssh_command(&ssh, config_path.as_deref(), &alias);
        // Same tab session as the file pane, so a password is asked once per tab.
        let askpass = app.state::<Arc<Broker>>().askpass(&session, &alias);
        for (name, value) in askpass.env() {
            cmd.env(name, value);
        }
        let exit_app = app.clone();
        let exit_events = events.clone();
        app.state::<Terminals>().start(|id| {
            PtySession::spawn(
                cmd,
                cols,
                rows,
                move |data| {
                    // Fails only once the frontend dropped the channel; the
                    // exit handler cleans up.
                    let _ = events.send(InvokeResponseBody::Raw(data.to_vec()));
                },
                move |status| {
                    exit_app.state::<Terminals>().remove(id);
                    let json = serde_json::to_string(&status).expect("Exit serializes");
                    let _ = exit_events.send(InvokeResponseBody::Json(json));
                },
            )
        })
    })
    .await
    .map_err(|e| format!("terminal task failed: {e}"))?
}

#[tauri::command]
pub fn terminal_write(
    terminals: State<'_, Terminals>,
    id: u32,
    data: String,
) -> Result<(), String> {
    terminals.get(id)?.write(data.as_bytes())
}

#[tauri::command]
pub fn terminal_resize(
    terminals: State<'_, Terminals>,
    id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    terminals.get(id)?.resize(cols, rows)
}

/// Stops the process. Its exit is still reported on the `exit` channel.
#[tauri::command]
pub fn terminal_close(terminals: State<'_, Terminals>, id: u32) {
    // Not found means it already exited.
    if let Ok(session) = terminals.get(id) {
        session.kill();
    }
}
