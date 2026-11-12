//! Tauri shell: exposes the Halcyon core to the web UI.
//!
//! Commands are thin wrappers around `halcyon-core`. Queries share one
//! database connection; a sync opens its own, which SQLite's WAL mode lets
//! run alongside the UI's reads.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use halcyon_core::autoconfig::{self, Candidate};
use halcyon_core::imap::{self, Credentials};
use halcyon_core::model::{Account, AuthMethod, FolderRole, MessageSummary, ServerConfig};
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
    /// Servers a freshly typed password may go to, per lower-cased address.
    ///
    /// The account dialog asks for the password only after the server check
    /// passed; this list makes the backend enforce the same order, so no
    /// front end can send a password to an unchecked host. IMAP entries got
    /// here by a successful connection (TLS certificate valid for the host);
    /// SMTP entries only if the provider published them, as there is no
    /// SMTP check yet.
    verified_servers: Mutex<HashMap<String, Vec<ServerConfig>>>,
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

#[derive(Serialize)]
struct DiscoverResult {
    candidates: Vec<Candidate>,
    reachable_index: Option<usize>,
    probe_error: Option<String>,
}

#[tauri::command]
fn list_accounts(state: State<'_, AppState>) -> CmdResult<Vec<Account>> {
    state.store.lock().map_err(err)?.accounts().map_err(err)
}

#[tauri::command]
async fn discover_account(state: State<'_, AppState>, email: String) -> CmdResult<DiscoverResult> {
    let candidates = autoconfig::discover(&email).await.map_err(err)?;
    let key = email.trim().to_lowercase();
    let published_smtp: Vec<ServerConfig> = candidates
        .iter()
        .filter(|c| c.source.is_authoritative())
        .filter_map(|c| c.smtp.clone())
        .collect();
    state
        .verified_servers
        .lock()
        .map_err(err)?
        .insert(key.clone(), published_smtp);
    let mut reachable_index = None;
    let mut first_error = None;

    for (idx, candidate) in candidates.iter().take(5).enumerate() {
        match imap::connect(&candidate.imap).await {
            Ok(mut client) => {
                let _ = client.run_command_and_check_ok("LOGOUT", None).await;
                reachable_index = Some(idx);
                mark_verified(&state, &key, &candidate.imap)?;
                break;
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e.to_string());
                }
            }
        }
    }

    Ok(DiscoverResult {
        candidates,
        reachable_index,
        probe_error: if reachable_index.is_some() {
            None
        } else {
            first_error
        },
    })
}

fn mark_verified(state: &State<'_, AppState>, email: &str, server: &ServerConfig) -> CmdResult<()> {
    let mut verified = state.verified_servers.lock().map_err(err)?;
    let list = verified.entry(email.trim().to_lowercase()).or_default();
    if !list.iter().any(|s| s.same_endpoint(server)) {
        list.push(server.clone());
    }
    Ok(())
}

/// Whether a freshly typed password may be sent to (or stored for) `server`:
/// it passed the check in this session, or the account already uses it.
fn may_receive_password(
    state: &State<'_, AppState>,
    email: &str,
    server: &ServerConfig,
) -> CmdResult<bool> {
    let key = email.trim().to_lowercase();
    if state
        .verified_servers
        .lock()
        .map_err(err)?
        .get(&key)
        .is_some_and(|list| list.iter().any(|s| s.same_endpoint(server)))
    {
        return Ok(true);
    }
    let existing = state
        .store
        .lock()
        .map_err(err)?
        .account_by_email(&key)
        .map_err(err)?;
    Ok(existing.is_some_and(|a| {
        a.imap.same_endpoint(server) || a.smtp.as_ref().is_some_and(|m| m.same_endpoint(server))
    }))
}

/// Checks that the server is reachable, speaks IMAP and presents a valid TLS
/// certificate for its name – without sending any credentials. Only servers
/// that pass may receive the password afterwards.
#[tauri::command]
async fn check_server(
    state: State<'_, AppState>,
    email: String,
    server: ServerConfig,
) -> CmdResult<String> {
    server.validate().map_err(err)?;
    let mut client = imap::connect(&server).await.map_err(err)?;
    let _ = client.run_command_and_check_ok("LOGOUT", None).await;
    mark_verified(&state, &email, &server)?;
    Ok(format!(
        "{} ist erreichbar und sicher verbunden.",
        server.host
    ))
}

#[tauri::command]
async fn save_account(
    state: State<'_, AppState>,
    mut account: Account,
    password: Option<String>,
) -> CmdResult<Account> {
    account.email = account.email.trim().to_string();
    account.display_name = account
        .display_name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let pw = password.filter(|s| !s.is_empty());

    account.imap.validate().map_err(err)?;
    if let Some(smtp) = &account.smtp {
        smtp.validate().map_err(err)?;
    }

    // Editing an account must not silently send the stored password to a
    // different server: changed connection settings require the password
    // again. The address is the keychain key and cannot change.
    if account.id != 0 {
        let existing = state
            .store
            .lock()
            .map_err(err)?
            .accounts()
            .map_err(err)?
            .into_iter()
            .find(|a| a.id == account.id)
            .ok_or_else(|| "Konto nicht gefunden.".to_string())?;
        account.email = existing.email.clone();
        let imap_changed = !existing.imap.same_endpoint(&account.imap);
        let smtp_changed = match (&existing.smtp, &account.smtp) {
            (Some(a), Some(b)) => !a.same_endpoint(b),
            (None, Some(_)) => true,
            _ => false,
        };
        if (imap_changed || smtp_changed) && pw.is_none() {
            return Err(
                "Die Servereinstellungen wurden geändert. Bitte gib das Passwort erneut ein, \
                 damit es nicht ungeprüft an einen anderen Server geht."
                    .into(),
            );
        }
    }

    if pw.is_some() && !may_receive_password(&state, &account.email, &account.imap)? {
        return Err(
            "Bitte zuerst den Server prüfen – erst danach wird das Passwort gesendet.".into(),
        );
    }
    // The SMTP password is only stored now and sent later; keep it only for
    // a server we have reason to trust: the IMAP host itself, a server the
    // provider published, or the one the account already uses.
    let smtp_trusted = match &account.smtp {
        Some(smtp) => {
            smtp.host.eq_ignore_ascii_case(&account.imap.host)
                || may_receive_password(&state, &account.email, smtp)?
        }
        None => false,
    };

    if let Some(ref pw) = pw
        && account.imap.auth == AuthMethod::Password
    {
        imap::probe(&account.imap, &Credentials::Password(pw.clone()))
            .await
            .map_err(err)?;
    } else if account.id == 0 {
        let mut client = imap::connect(&account.imap).await.map_err(err)?;
        let _ = client.run_command_and_check_ok("LOGOUT", None).await;
    }

    let saved = {
        let store = state.store.lock().map_err(err)?;
        if account.id == 0 {
            if store
                .account_by_email(&account.email)
                .map_err(err)?
                .is_some()
            {
                return Err(format!("Konto {} ist bereits eingerichtet.", account.email));
            }
            store.add_account(&account).map_err(err)?
        } else {
            store.update_account(&account).map_err(err)?;
            account
        }
    };

    if let Some(pw) = pw {
        let email = saved.email.clone();
        let has_smtp = smtp_trusted;
        tauri::async_runtime::spawn_blocking(move || -> halcyon_core::Result<()> {
            secrets::store(&email, SecretKind::ImapPassword, &pw)?;
            if has_smtp {
                secrets::store(&email, SecretKind::SmtpPassword, &pw)?;
            }
            Ok(())
        })
        .await
        .map_err(err)?
        .map_err(err)?;
    }

    Ok(saved)
}

#[tauri::command]
async fn delete_account(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    let email = {
        let store = state.store.lock().map_err(err)?;
        let accounts = store.accounts().map_err(err)?;
        let acc = accounts
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| "Konto nicht gefunden.".to_string())?;
        store.remove_account(id).map_err(err)?;
        acc.email
    };

    tauri::async_runtime::spawn_blocking(move || -> halcyon_core::Result<()> {
        secrets::delete(&email, SecretKind::ImapPassword)?;
        secrets::delete(&email, SecretKind::SmtpPassword)?;
        Ok(())
    })
    .await
    .map_err(err)?
    .map_err(err)?;

    Ok(())
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
            verified_servers: Mutex::new(HashMap::new()),
        })
        .invoke_handler(tauri::generate_handler![
            overview,
            messages,
            search,
            start_sync,
            list_accounts,
            discover_account,
            check_server,
            save_account,
            delete_account
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Halcyon app");
}
