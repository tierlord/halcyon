//! Sending in the background. Messages wait in the outbox for the "undo"
//! window, then a single worker delivers them one by one, files the sent
//! copy and marks the original as answered. Offline or failing servers are
//! retried with growing pauses; rejected messages stay visible as failed.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use halcyon_core::imap::Credentials;
use halcyon_core::model::{Account, FolderRole};
use halcyon_core::secrets::{self, SecretKind};
use halcyon_core::send;
use halcyon_core::store::{OutboxItem, Store};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::AppState;

/// How long a message waits before it really leaves ("Rückgängig").
pub const UNDO_SECONDS: i64 = 10;
const MAX_ATTEMPTS: u32 = 6;
/// Upper bound for sleeping, so a clock change cannot stall the queue.
const MAX_SLEEP: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Outbox {
    pub wake: Notify,
}

#[derive(Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OutboxEvent {
    Sending {
        id: i64,
        subject: String,
    },
    Sent {
        id: i64,
        subject: String,
    },
    Retrying {
        id: i64,
        subject: String,
        error: String,
        retry_in: i64,
    },
    Failed {
        id: i64,
        subject: String,
        error: String,
    },
    /// Sent, but the copy for the sent folder could not be filed.
    SentCopyFailed {
        id: i64,
        error: String,
    },
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Starts the single delivery worker.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let db_path = app.state::<AppState>().db_path.clone();
        let mut store = match Store::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "outbox worker cannot open the database");
                return;
            }
        };
        let _ = store.requeue_interrupted_outgoing();
        loop {
            match store.claim_due_outgoing(now()) {
                Ok(Some(item)) => {
                    process(&app, &mut store, item).await;
                    continue;
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "outbox query failed"),
            }
            let sleep = match store.next_outgoing_due() {
                Ok(Some(due)) => Duration::from_secs((due - now()).max(0) as u64).min(MAX_SLEEP),
                _ => MAX_SLEEP,
            };
            let outbox = &app.state::<AppState>().outbox;
            tokio::select! {
                _ = tokio::time::sleep(sleep) => {}
                _ = outbox.wake.notified() => {}
            }
        }
    });
}

// `&mut Store` keeps the futures `Send` (the connection is not `Sync`).
async fn process(app: &AppHandle, store: &mut Store, item: OutboxItem) {
    let emit = |e: OutboxEvent| {
        let _ = app.emit("outbox", e);
    };
    emit(OutboxEvent::Sending {
        id: item.id,
        subject: item.subject.clone(),
    });

    let account = store
        .accounts()
        .ok()
        .and_then(|a| a.into_iter().find(|a| a.id == item.account_id));
    let Some(account) = account else {
        let _ = store.fail_outgoing(item.id, "Konto nicht gefunden", None);
        return emit(OutboxEvent::Failed {
            id: item.id,
            subject: item.subject,
            error: "Konto nicht gefunden".into(),
        });
    };

    match deliver(&account, &item).await {
        Ok(()) => {
            let _ = store.finish_outgoing(item.id);
            emit(OutboxEvent::Sent {
                id: item.id,
                subject: item.subject.clone(),
            });
            after_sent(app, store, &account, &item).await;
        }
        Err(e) if e.permanent || item.attempts >= MAX_ATTEMPTS => {
            let _ = store.fail_outgoing(item.id, &e.message, None);
            emit(OutboxEvent::Failed {
                id: item.id,
                subject: item.subject,
                error: e.message,
            });
        }
        Err(e) => {
            // 30 s, 1 min, 2 min, 4 min, … – the server or the network may
            // just be away for a moment.
            let retry_in = 30_i64 << (item.attempts.saturating_sub(1)).min(5);
            let _ = store.fail_outgoing(item.id, &e.message, Some(now() + retry_in));
            emit(OutboxEvent::Retrying {
                id: item.id,
                subject: item.subject,
                error: e.message,
                retry_in,
            });
        }
    }
}

async fn deliver(account: &Account, item: &OutboxItem) -> Result<(), send::DeliveryError> {
    let fail = |message: &str, permanent| send::DeliveryError {
        message: message.to_string(),
        permanent,
    };
    let smtp = account.smtp.as_ref().ok_or_else(|| {
        fail(
            "Für dieses Konto ist kein Postausgangsserver eingerichtet.",
            true,
        )
    })?;
    let password = smtp_password(account)
        .await
        .ok_or_else(|| fail("Kein Passwort für den Postausgangsserver gespeichert. Bitte im Konto-Dialog das Passwort erneut eingeben.", true))?;
    send::deliver(smtp, &password, &item.built).await
}

/// The stored SMTP password; for accounts set up before SMTP passwords were
/// kept separately, the IMAP password when both use the same login.
async fn smtp_password(account: &Account) -> Option<String> {
    let email = account.email.clone();
    let same_login = account
        .smtp
        .as_ref()
        .is_some_and(|s| s.username == account.imap.username);
    tauri::async_runtime::spawn_blocking(move || {
        secrets::load(&email, SecretKind::SmtpPassword)
            .ok()
            .flatten()
            .or_else(|| {
                same_login
                    .then(|| {
                        secrets::load(&email, SecretKind::ImapPassword)
                            .ok()
                            .flatten()
                    })
                    .flatten()
            })
    })
    .await
    .ok()
    .flatten()
}

/// Marks the original answered and files the sent copy. Failures here do
/// not undo the sending; the user is told about a missing sent copy.
async fn after_sent(app: &AppHandle, store: &mut Store, account: &Account, item: &OutboxItem) {
    if let Some(original) = item.draft.reply_to_local_id {
        let state = app.state::<AppState>();
        if let Ok(mut s) = state.store.lock() {
            let _ = s.queue_flag(&[original], "\\Answered", true);
        }
        crate::schedule_flush(app);
    }

    let sent_folder = store.folders(account.id).ok().and_then(|f| {
        f.into_iter()
            .find(|f| f.role == Some(FolderRole::Sent))
            .map(|f| f.path)
    });
    let Some(sent_folder) = sent_folder else {
        let _ = app.emit(
            "outbox",
            OutboxEvent::SentCopyFailed {
                id: item.id,
                error: "kein Gesendet-Ordner gefunden".into(),
            },
        );
        return;
    };
    let email = account.email.clone();
    let password = tauri::async_runtime::spawn_blocking(move || {
        secrets::load(&email, SecretKind::ImapPassword)
    })
    .await;
    let Ok(Ok(Some(password))) = password else {
        return;
    };
    if let Err(e) = send::file_sent_copy(
        account,
        &Credentials::Password(password),
        &sent_folder,
        &item.built.record,
    )
    .await
    {
        let _ = app.emit(
            "outbox",
            OutboxEvent::SentCopyFailed {
                id: item.id,
                error: e.to_string(),
            },
        );
    }
}
