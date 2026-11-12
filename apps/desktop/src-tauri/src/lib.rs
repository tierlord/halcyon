//! Tauri shell: exposes the Halcyon core to the web UI.
//!
//! Commands are thin wrappers around `halcyon-core`. Queries share one
//! database connection; a sync opens its own, which SQLite's WAL mode lets
//! run alongside the UI's reads.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use halcyon_core::imap::Credentials;
use halcyon_core::model::{FolderRole, MessageSummary};
use halcyon_core::secrets::{self, SecretKind};
use halcyon_core::store::Store;
use halcyon_core::sync::{self, SyncEvent};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

const LIST_LIMIT: usize = 500;

struct AppState {
    db_path: PathBuf,
    store: Mutex<Store>,
    syncing: AtomicBool,
}

/// Errors cross the IPC boundary as plain strings.
type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
struct AccountInfo {
    id: i64,
    email: String,
    display_name: Option<String>,
    unread_inbox: i64,
}

#[derive(Serialize)]
struct Overview {
    accounts: Vec<AccountInfo>,
    unread_inbox: i64,
}

#[tauri::command]
fn overview(state: State<'_, AppState>) -> CmdResult<Overview> {
    let store = state.store.lock().map_err(err)?;
    let accounts = store
        .accounts()
        .map_err(err)?
        .into_iter()
        .map(|a| {
            Ok(AccountInfo {
                unread_inbox: store.unread_count(Some(a.id), Some(FolderRole::Inbox))?,
                id: a.id,
                email: a.email,
                display_name: a.display_name,
            })
        })
        .collect::<halcyon_core::Result<_>>()
        .map_err(err)?;
    Ok(Overview {
        accounts,
        unread_inbox: store
            .unread_count(None, Some(FolderRole::Inbox))
            .map_err(err)?,
    })
}

#[tauri::command]
fn messages(
    state: State<'_, AppState>,
    account_id: Option<i64>,
    role: Option<String>,
) -> CmdResult<Vec<MessageSummary>> {
    let role = role.as_deref().and_then(FolderRole::parse);
    state
        .store
        .lock()
        .map_err(err)?
        .messages(account_id, role, LIST_LIMIT)
        .map_err(err)
}

#[tauri::command]
fn search(state: State<'_, AppState>, query: String) -> CmdResult<Vec<MessageSummary>> {
    state
        .store
        .lock()
        .map_err(err)?
        .search(&query, LIST_LIMIT)
        .map_err(err)
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SyncUpdate {
    Started,
    Account {
        email: String,
    },
    Folder {
        email: String,
        path: String,
        done: usize,
        total: usize,
    },
    Failed {
        email: String,
        error: String,
    },
    Finished {
        new_messages: usize,
    },
}

/// Starts syncing all accounts in the background. Progress arrives as
/// `sync` events; a second call while a sync runs is ignored.
#[tauri::command]
fn start_sync(app: AppHandle, state: State<'_, AppState>) -> CmdResult<bool> {
    if state.syncing.swap(true, Ordering::SeqCst) {
        return Ok(false);
    }
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn(async move {
        let new_messages = run_sync(&app, &db_path).await;
        let _ = app.emit("sync", SyncUpdate::Finished { new_messages });
        app.state::<AppState>()
            .syncing
            .store(false, Ordering::SeqCst);
    });
    Ok(true)
}

async fn run_sync(app: &AppHandle, db_path: &std::path::Path) -> usize {
    let emit = |u: SyncUpdate| {
        let _ = app.emit("sync", u);
    };
    emit(SyncUpdate::Started);
    let mut store = match Store::open(db_path) {
        Ok(s) => s,
        Err(e) => {
            emit(SyncUpdate::Failed {
                email: String::new(),
                error: e.to_string(),
            });
            return 0;
        }
    };
    let accounts = store.accounts().unwrap_or_default();
    let mut total_new = 0;
    for account in accounts {
        let email = account.email.clone();
        emit(SyncUpdate::Account {
            email: email.clone(),
        });
        // The keychain call blocks on D-Bus.
        let lookup_email = email.clone();
        let password = tauri::async_runtime::spawn_blocking(move || {
            secrets::load(&lookup_email, SecretKind::ImapPassword)
        })
        .await;
        let password = match password {
            Ok(Ok(Some(p))) => p,
            Ok(Ok(None)) => {
                emit(SyncUpdate::Failed {
                    email,
                    error: "Kein Passwort im Schlüsselbund".into(),
                });
                continue;
            }
            Ok(Err(e)) => {
                emit(SyncUpdate::Failed {
                    email,
                    error: e.to_string(),
                });
                continue;
            }
            Err(e) => {
                emit(SyncUpdate::Failed {
                    email,
                    error: e.to_string(),
                });
                continue;
            }
        };
        let result = sync::sync_account(
            &mut store,
            &account,
            &Credentials::Password(password),
            |event| match event {
                SyncEvent::FolderStarted { path, new_messages } if new_messages > 0 => {
                    emit(SyncUpdate::Folder {
                        email: email.clone(),
                        path: path.to_string(),
                        done: 0,
                        total: new_messages,
                    })
                }
                SyncEvent::Progress { path, done, total } => emit(SyncUpdate::Folder {
                    email: email.clone(),
                    path: path.to_string(),
                    done,
                    total,
                }),
                SyncEvent::FolderFailed { path, error } => emit(SyncUpdate::Failed {
                    email: email.clone(),
                    error: format!("{path}: {error}"),
                }),
                _ => {}
            },
        )
        .await;
        match result {
            Ok(report) => total_new += report.new_messages,
            Err(e) => emit(SyncUpdate::Failed {
                email: email.clone(),
                error: e.to_string(),
            }),
        }
    }
    total_new
}

fn default_db_path() -> PathBuf {
    if let Some(p) = std::env::var_os("HALCYON_DB") {
        return p.into();
    }
    directories::ProjectDirs::from("", "", "halcyon")
        .map(|d| d.data_dir().join("halcyon.db"))
        .unwrap_or_else(|| PathBuf::from("halcyon.db"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("HALCYON_LOG")
                .unwrap_or_else(|_| "warn,rustls_platform_verifier=off".into()),
        )
        .init();

    let db_path = default_db_path();
    let store = Store::open(&db_path).expect("could not open the Halcyon database");

    tauri::Builder::default()
        .manage(AppState {
            db_path,
            store: Mutex::new(store),
            syncing: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            overview, messages, search, start_sync
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Halcyon app");
}
