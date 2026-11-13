//! Live mail: one background watcher per account keeps a connection to the
//! inbox open and syncs it as soon as the server reports a change (IMAP
//! IDLE). Servers without IDLE are checked every few minutes instead.
//!
//! Watchers reconnect on their own with growing pauses, so a laptop that
//! sleeps or changes networks recovers without user action.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use halcyon_core::imap::{self, Credentials};
use halcyon_core::model::{Account, FolderRole, MessageSummary};
use halcyon_core::secrets::{self, SecretKind};
use halcyon_core::store::Store;
use halcyon_core::sync;
use serde::Serialize;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::AppState;

/// Check interval for servers without IDLE.
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
const BACKOFF_START: Duration = Duration::from_secs(20);
const BACKOFF_MAX: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
pub struct Watchers {
    tasks: Mutex<HashMap<i64, JoinHandle<()>>>,
}

#[derive(Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LiveState {
    /// Connected; the server pushes changes.
    Push { email: String },
    /// Connected; the server has no IDLE, so it is checked regularly.
    Polling { email: String },
    /// Not connected; retrying.
    Offline { email: String, error: String },
}

/// A watcher is syncing an account right now (for the sidebar animation).
#[derive(Clone, Serialize)]
pub struct Activity {
    pub email: String,
    pub active: bool,
}

/// Something changed in an account's inbox.
#[derive(Clone, Serialize)]
pub struct MailChanged {
    pub account_id: i64,
    pub new_messages: usize,
}

/// (Re)starts one watcher per account, e.g. at launch or after accounts
/// were added or removed.
pub fn restart(app: &AppHandle) {
    let state = app.state::<AppState>();
    let accounts = match state.store.lock() {
        Ok(store) => store.accounts().unwrap_or_default(),
        Err(_) => return,
    };
    let Ok(mut tasks) = state.watchers.tasks.lock() else {
        return;
    };
    for (_, task) in tasks.drain() {
        task.abort();
    }
    for account in accounts {
        let app = app.clone();
        let id = account.id;
        tasks.insert(
            id,
            tauri::async_runtime::spawn(async move { watch(app, account).await }),
        );
    }
}

async fn watch(app: AppHandle, account: Account) {
    let mut backoff = BACKOFF_START;
    loop {
        let error = match watch_connected(&app, &account).await {
            Ok(()) => "Verbindung beendet".to_string(),
            Err(e) => e,
        };
        let _ = app.emit(
            "live",
            LiveState::Offline {
                email: account.email.clone(),
                error,
            },
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// Runs until the connection breaks.
async fn watch_connected(app: &AppHandle, account: &Account) -> Result<(), String> {
    let email = account.email.clone();
    let password = tauri::async_runtime::spawn_blocking(move || {
        secrets::load(&email, SecretKind::ImapPassword)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?
    .ok_or("kein Passwort im Schlüsselbund")?;

    let (mut session, caps) = imap::login(&account.imap, &Credentials::Password(password))
        .await
        .map_err(|e| e.to_string())?;
    let db_path = app.state::<AppState>().db_path.clone();
    let mut store = Store::open(&db_path).map_err(|e| e.to_string())?;

    let live = if caps.idle {
        LiveState::Push {
            email: account.email.clone(),
        }
    } else {
        LiveState::Polling {
            email: account.email.clone(),
        }
    };
    let _ = app.emit("live", live);

    loop {
        let before = newest_inbox_id(&store, account.id);
        let activity = |active| {
            let _ = app.emit(
                "activity",
                Activity {
                    email: account.email.clone(),
                    active,
                },
            );
        };
        activity(true);
        let synced = sync::sync_inbox(&mut store, &mut session, &caps, account).await;
        activity(false);
        match synced.map_err(|e| e.to_string())? {
            Some(report) => {
                if report.new_messages + report.removed + report.flags_updated > 0 {
                    let _ = app.emit(
                        "mail-changed",
                        MailChanged {
                            account_id: account.id,
                            new_messages: report.new_messages,
                        },
                    );
                }
                if report.new_messages > 0 {
                    notify_new(app, &store, account, before);
                }
            }
            // Never fully synced yet: the regular sync creates the folders.
            None => tokio::time::sleep(POLL_INTERVAL).await,
        }

        if caps.idle {
            // sync_inbox leaves the inbox selected.
            let (s, _changed) = sync::idle_wait(session, sync::IDLE_RENEW)
                .await
                .map_err(|e| e.to_string())?;
            session = s;
        } else {
            tokio::time::sleep(POLL_INTERVAL).await;
            // Keeps the connection alive and notices a dead one early.
            session.noop().await.map_err(|e| e.to_string())?;
        }
    }
}

fn newest_inbox_id(store: &Store, account_id: i64) -> i64 {
    store
        .messages(Some(account_id), Some(FolderRole::Inbox), 1)
        .ok()
        .and_then(|m| m.first().map(|m| m.id))
        .unwrap_or(0)
}

/// Shows a desktop notification for new unread messages – not while the
/// Halcyon window has the focus, where the list updates anyway.
fn notify_new(app: &AppHandle, store: &Store, account: &Account, before: i64) {
    let focused = app
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    if focused {
        return;
    }
    let fresh: Vec<MessageSummary> = store
        .messages(Some(account.id), Some(FolderRole::Inbox), 50)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.id > before && !m.is_seen())
        .collect();
    let (title, body) = match fresh.as_slice() {
        [] => return,
        [m] => (
            sender(m),
            m.header
                .subject
                .clone()
                .unwrap_or_else(|| "(kein Betreff)".into()),
        ),
        many => (
            format!("{} neue Nachrichten", many.len()),
            many.iter()
                .take(3)
                .map(sender)
                .collect::<Vec<_>>()
                .join(", "),
        ),
    };
    let _ = app.notification().builder().title(title).body(body).show();
}

fn sender(m: &MessageSummary) -> String {
    m.header
        .from_name
        .clone()
        .or_else(|| m.header.from_addr.clone())
        .unwrap_or_else(|| "Unbekannt".into())
}
