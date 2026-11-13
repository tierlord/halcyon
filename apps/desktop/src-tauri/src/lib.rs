//! Tauri shell: exposes the Halcyon core to the web UI.
//!
//! Commands are thin wrappers around `halcyon-core`. Queries share one
//! database connection; a sync opens its own, which SQLite's WAL mode lets
//! run alongside the UI's reads.

mod outbox;
mod watch;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use halcyon_core::autoconfig::{self, Candidate};
use halcyon_core::brand::{self, BrandLogo};
use halcyon_core::cases::{CaseStatus, CaseView};
use halcyon_core::content::{self, Attachment, Rendered};
use halcyon_core::imap::{self, Credentials};
use halcyon_core::linker;
use halcyon_core::model::{Account, AuthMethod, FolderRole, MessageSummary, ServerConfig};
use halcyon_core::secrets::{self, SecretKind};
use halcyon_core::store::{CachedLogo, Store};
use halcyon_core::sync::{self, SyncEvent};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

const LIST_LIMIT: usize = 500;

struct AppState {
    db_path: PathBuf,
    store: Mutex<Store>,
    syncing: AtomicBool,
    /// A background run sending queued local changes is active …
    flushing: AtomicBool,
    /// … and more changes arrived while it ran.
    flush_again: AtomicBool,
    watchers: watch::Watchers,
    outbox: outbox::Outbox,
    logo_semaphore: tokio::sync::Semaphore,
    /// The case linker is running …
    linking: AtomicBool,
    /// … and new mail arrived meanwhile.
    link_again: AtomicBool,
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
struct FolderInfo {
    id: i64,
    path: String,
    name: String,
    depth: usize,
    role: Option<FolderRole>,
    selectable: bool,
    unread: i64,
    total: i64,
}

#[derive(Serialize)]
struct AccountInfo {
    id: i64,
    email: String,
    display_name: Option<String>,
    unread_inbox: i64,
    folders: Vec<FolderInfo>,
}

#[derive(Serialize)]
struct Overview {
    accounts: Vec<AccountInfo>,
    unread_inbox: i64,
}

fn folder_display_and_depth(
    path: &str,
    delimiter: Option<&str>,
    role: Option<FolderRole>,
) -> (String, usize) {
    let decoded = halcyon_core::utf7::decode(path);
    if role == Some(FolderRole::Inbox) && path.eq_ignore_ascii_case("INBOX") {
        return ("Posteingang".to_string(), 0);
    }
    let depth = delimiter
        .filter(|d| !d.is_empty())
        .map(|d| decoded.split(d).count().saturating_sub(1))
        .unwrap_or(0);
    // Known folders get one German name, whatever the server calls them
    // ("Sent Items", "Gesendete Objekte", …).
    let german = match role {
        Some(FolderRole::Sent) => Some("Gesendet"),
        Some(FolderRole::Drafts) => Some("Entwürfe"),
        Some(FolderRole::Archive) => Some("Archiv"),
        Some(FolderRole::Junk) => Some("Spam"),
        Some(FolderRole::Trash) => Some("Papierkorb"),
        Some(FolderRole::All) => Some("Alle Nachrichten"),
        Some(FolderRole::Flagged) => Some("Markiert"),
        _ => None,
    };
    if let Some(name) = german {
        return (name.to_string(), depth);
    }
    if let Some(delim) = delimiter.filter(|d| !d.is_empty()) {
        let parts: Vec<&str> = decoded.split(delim).collect();
        if parts.len() > 1 {
            let depth = parts.len() - 1;
            let name = parts
                .last()
                .copied()
                .filter(|s| !s.is_empty())
                .unwrap_or(&decoded);
            return (name.to_string(), depth);
        }
    }
    (decoded, 0)
}

/// Two folders with the same German name (a server keeping "Sent" and
/// "Sent Items") show the server's name too, so the user can tell them apart.
fn disambiguate(folders: &mut [FolderInfo]) {
    let names: Vec<String> = folders.iter().map(|f| f.name.clone()).collect();
    for f in folders.iter_mut() {
        if names.iter().filter(|n| **n == f.name).count() > 1 {
            let leaf = halcyon_core::utf7::decode(&f.path);
            let leaf = leaf.rsplit(['/', '.']).next().unwrap_or(&leaf).to_string();
            if leaf != f.name {
                f.name = format!("{} ({leaf})", f.name);
            }
        }
    }
}

fn folder_sort_key(folder: &FolderInfo) -> (u8, String) {
    let rank = match folder.role {
        Some(FolderRole::Inbox) => 0,
        Some(FolderRole::Sent) => 1,
        Some(FolderRole::Drafts) => 2,
        Some(FolderRole::Archive) => 3,
        Some(FolderRole::Junk) => 4,
        Some(FolderRole::Trash) => 5,
        Some(FolderRole::All) => 6,
        Some(FolderRole::Flagged) => 7,
        None => 8,
    };
    (rank, folder.path.to_lowercase())
}

#[tauri::command]
fn overview(state: State<'_, AppState>) -> CmdResult<Overview> {
    let store = state.store.lock().map_err(err)?;
    let unread_by_folder = store.unread_counts_by_folder().map_err(err)?;
    let accounts = store
        .accounts()
        .map_err(err)?
        .into_iter()
        .map(|a| {
            let mut folders: Vec<FolderInfo> = store
                .folders(a.id)?
                .into_iter()
                .map(|f| {
                    let (name, depth) =
                        folder_display_and_depth(&f.path, f.delimiter.as_deref(), f.role);
                    let unread = unread_by_folder.get(&f.id).copied().unwrap_or(0);
                    let total = store.message_count(f.id).unwrap_or(0);
                    FolderInfo {
                        id: f.id,
                        path: f.path,
                        name,
                        depth,
                        role: f.role,
                        selectable: f.selectable,
                        unread,
                        total,
                    }
                })
                .collect();
            folders.sort_by_key(folder_sort_key);
            disambiguate(&mut folders);
            Ok(AccountInfo {
                unread_inbox: store.unread_count(Some(a.id), Some(FolderRole::Inbox))?,
                id: a.id,
                email: a.email,
                display_name: a.display_name,
                folders,
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
    folder_id: Option<i64>,
) -> CmdResult<Vec<MessageSummary>> {
    let store = state.store.lock().map_err(err)?;
    if let Some(fid) = folder_id {
        return store.folder_messages(fid, LIST_LIMIT).map_err(err);
    }
    let role = role.as_deref().and_then(FolderRole::parse);
    store.messages(account_id, role, LIST_LIMIT).map_err(err)
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
    app: AppHandle,
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
            let known = smtp.host.eq_ignore_ascii_case(&account.imap.host)
                || may_receive_password(&state, &account.email, smtp)?;
            // Like the IMAP check: reachable with a valid certificate, no
            // password sent. Only then the password is kept for it.
            known
                || (pw.is_some()
                    && halcyon_core::send::check_smtp(smtp)
                        .await
                        .map_err(|e| format!("Postausgangsserver: {e}"))
                        .is_ok())
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

    // The new or changed account gets a live watcher, and a first sync
    // creates its folders.
    watch::restart(&app);
    begin_sync(&app);
    Ok(saved)
}

#[tauri::command]
async fn delete_account(app: AppHandle, state: State<'_, AppState>, id: i64) -> CmdResult<()> {
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

    watch::restart(&app);
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
    Bodies {
        email: String,
        path: String,
        done: usize,
        total: usize,
    },
    Finished {
        new_messages: usize,
    },
}

/// Starts syncing all accounts in the background. Progress arrives as
/// `sync` events; a second call while a sync runs is ignored.
#[tauri::command]
fn start_sync(app: AppHandle) -> CmdResult<bool> {
    Ok(begin_sync(&app))
}

/// Starts a full sync of all accounts unless one is running.
fn begin_sync(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    if state.syncing.swap(true, Ordering::SeqCst) {
        return false;
    }
    let app = app.clone();
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn(async move {
        let new_messages = run_sync(&app, &db_path).await;
        let _ = app.emit("sync", SyncUpdate::Finished { new_messages });
        schedule_link(&app);
        let state = app.state::<AppState>();
        state.syncing.store(false, Ordering::SeqCst);
        // Changes made while the sync ran were not part of its first step.
        let pending = state
            .store
            .lock()
            .map(|s| s.has_pending_ops().unwrap_or(false))
            .unwrap_or(false);
        if pending {
            schedule_flush(&app);
        }
    });
    true
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
                SyncEvent::Bodies { path, done, total } => emit(SyncUpdate::Bodies {
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

#[derive(Serialize)]
struct MessageView {
    rendered: Rendered,
    attachments: Vec<Attachment>,
}

/// The raw message from the store, or downloaded now if it is not there yet.
async fn raw_message(state: &State<'_, AppState>, message_id: i64) -> CmdResult<Vec<u8>> {
    let (stored, message, account) = {
        let store = state.store.lock().map_err(err)?;
        let stored = store.body(message_id).map_err(err)?;
        let message = store.message(message_id).map_err(err)?;
        let account = match &message {
            Some(m) => store
                .accounts()
                .map_err(err)?
                .into_iter()
                .find(|a| a.id == m.account_id),
            None => None,
        };
        (stored, message, account)
    };
    if let Some(raw) = stored {
        return Ok(raw);
    }
    let message = message.ok_or("Nachricht nicht gefunden.")?;
    let account = account.ok_or("Konto nicht gefunden.")?;
    let email = account.email.clone();
    let password = tauri::async_runtime::spawn_blocking(move || {
        secrets::load(&email, SecretKind::ImapPassword)
    })
    .await
    .map_err(err)?
    .map_err(err)?
    .ok_or("Kein Passwort im Schlüsselbund.")?;
    let raw = sync::download_message(
        &account,
        &Credentials::Password(password),
        &message.folder_path,
        message.header.uid,
    )
    .await
    .map_err(err)?;
    state
        .store
        .lock()
        .map_err(err)?
        .store_body(message_id, &raw)
        .map_err(err)?;
    Ok(raw)
}

/// Content of a message as a sandbox-ready document. Remote images load only
/// with `allow_remote`, which the user sets per message.
#[tauri::command]
async fn message_content(
    state: State<'_, AppState>,
    id: i64,
    allow_remote: bool,
    dark: bool,
) -> CmdResult<MessageView> {
    let raw = raw_message(&state, id).await?;
    let parsed = content::parse(&raw);
    Ok(MessageView {
        rendered: content::render(&parsed, allow_remote, dark),
        attachments: parsed.attachments.clone(),
    })
}

#[derive(Serialize)]
struct ComposeSeed {
    account_id: i64,
    draft: halcyon_core::reply::DraftSeed,
}

/// Pre-fills a reply, reply-all or forward for a message.
#[tauri::command]
async fn compose_reply(
    state: State<'_, AppState>,
    id: i64,
    kind: halcyon_core::reply::ReplyKind,
    utc_offset_minutes: i32,
) -> CmdResult<ComposeSeed> {
    let raw = raw_message(&state, id).await?;
    let text = content::parse(&raw).text.unwrap_or_default();
    let (message, own) = {
        let store = state.store.lock().map_err(err)?;
        let message = store
            .message(id)
            .map_err(err)?
            .ok_or("Nachricht nicht gefunden.")?;
        let own: Vec<String> = store
            .accounts()
            .map_err(err)?
            .into_iter()
            .map(|a| a.email.to_lowercase())
            .collect();
        (message, own)
    };
    Ok(ComposeSeed {
        account_id: message.account_id,
        draft: halcyon_core::reply::prepare(kind, &message.header, &text, &own, utc_offset_minutes),
    })
}

#[derive(Serialize)]
struct SendTicket {
    id: i64,
    send_after: i64,
}

/// Builds the message and puts it into the outbox. It leaves after the undo
/// window unless [`cancel_send`] takes it back.
#[tauri::command]
fn send_message(
    state: State<'_, AppState>,
    draft: halcyon_core::send::Draft,
) -> CmdResult<SendTicket> {
    let store = state.store.lock().map_err(err)?;
    let account = store
        .accounts()
        .map_err(err)?
        .into_iter()
        .find(|a| a.id == draft.account_id)
        .ok_or("Konto nicht gefunden.")?;
    if account.smtp.is_none() {
        return Err("Für dieses Konto ist kein Postausgangsserver eingerichtet.".into());
    }
    let built =
        halcyon_core::send::build(&draft, &account, std::time::SystemTime::now()).map_err(|e| {
            match e {
                halcyon_core::Error::InvalidAddress(a) => format!("Ungültige Adresse: {a}"),
                halcyon_core::Error::Other(m) if m == "no recipients" => {
                    "Bitte mindestens einen Empfänger angeben.".into()
                }
                e => e.to_string(),
            }
        })?;
    let send_after = outbox::now() + outbox::UNDO_SECONDS;
    let id = store
        .queue_outgoing(&draft, &built, send_after)
        .map_err(err)?;
    drop(store);
    state.outbox.wake.notify_one();
    Ok(SendTicket { id, send_after })
}

/// "Rückgängig": takes a message back while it is still waiting. Returns its
/// draft for the editor, or `None` if it already left.
#[tauri::command]
fn cancel_send(
    state: State<'_, AppState>,
    id: i64,
) -> CmdResult<Option<halcyon_core::send::Draft>> {
    state
        .store
        .lock()
        .map_err(err)?
        .cancel_outgoing(id)
        .map_err(err)
}

/// A message that failed for good, back to the editor.
#[tauri::command]
fn take_failed(
    state: State<'_, AppState>,
    id: i64,
) -> CmdResult<Option<halcyon_core::send::Draft>> {
    state
        .store
        .lock()
        .map_err(err)?
        .take_failed_outgoing(id)
        .map_err(err)
}

#[tauri::command]
fn outbox_list(state: State<'_, AppState>) -> CmdResult<Vec<halcyon_core::store::OutboxEntry>> {
    state.store.lock().map_err(err)?.outbox().map_err(err)
}

/// Opens a link from a message in the system browser or mail handler.
/// Only web and mail links: anything else (file:, custom app schemes) could
/// start programs.
#[tauri::command]
fn open_link(url: String) -> CmdResult<()> {
    let lower = url.trim().to_ascii_lowercase();
    if !(lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:"))
    {
        return Err("Dieser Link-Typ wird nicht geöffnet.".into());
    }
    tauri_plugin_opener::open_url(url.trim(), None::<&str>).map_err(err)
}

/// Saves an attachment to the downloads folder (never overwriting) and shows
/// it in the file manager. Attachments are not opened directly, so a
/// disguised program cannot start with one click.
#[tauri::command]
async fn save_attachment(state: State<'_, AppState>, id: i64, part: u32) -> CmdResult<String> {
    let raw = raw_message(&state, id).await?;
    let parsed = content::parse(&raw);
    let info = parsed
        .attachments
        .iter()
        .find(|a| a.part == part)
        .ok_or("Anhang nicht gefunden.")?;
    let data = content::attachment_data(&raw, part).ok_or("Anhang nicht lesbar.")?;
    let dir = directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(|p| p.to_path_buf()))
        .ok_or("Kein Download-Ordner gefunden.")?;
    let path = unique_path(&dir, &content::safe_file_name(&info.name));
    std::fs::write(&path, data).map_err(err)?;
    let _ = tauri_plugin_opener::reveal_item_in_dir(&path);
    Ok(path.display().to_string())
}

/// `name.pdf`, else `name (2).pdf`, `name (3).pdf`, …
fn unique_path(dir: &std::path::Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

/// Maps the UI's flag names to IMAP system flags. Only these two can be set
/// from the UI.
fn imap_flag(name: &str) -> CmdResult<&'static str> {
    match name {
        "seen" => Ok("\\Seen"),
        "flagged" => Ok("\\Flagged"),
        _ => Err(format!("Unbekannte Markierung {name:?}.")),
    }
}

/// Marks messages read/unread or flagged/unflagged. Takes effect locally at
/// once; the server follows in the background.
#[tauri::command]
fn set_flag(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<i64>,
    flag: String,
    on: bool,
) -> CmdResult<usize> {
    let changed = state
        .store
        .lock()
        .map_err(err)?
        .queue_flag(&ids, imap_flag(&flag)?, on)
        .map_err(err)?;
    if changed > 0 {
        schedule_flush(&app);
    }
    Ok(changed)
}

/// Marks every message of the given folders read (one server command per
/// folder).
#[tauri::command]
fn mark_folders_read(
    app: AppHandle,
    state: State<'_, AppState>,
    folder_ids: Vec<i64>,
) -> CmdResult<usize> {
    let mut changed = 0;
    {
        let mut store = state.store.lock().map_err(err)?;
        for id in folder_ids {
            changed += store.queue_mark_folder_seen(id).map_err(err)?;
        }
    }
    if changed > 0 {
        schedule_flush(&app);
    }
    Ok(changed)
}

/// Empties a spam or trash folder for good.
#[tauri::command]
fn empty_folder(app: AppHandle, state: State<'_, AppState>, folder_id: i64) -> CmdResult<usize> {
    let removed = state
        .store
        .lock()
        .map_err(err)?
        .queue_empty_folder(folder_id)
        .map_err(|e| match e {
            halcyon_core::Error::Other(m) if m.contains("only spam") => {
                "Nur Spam und Papierkorb lassen sich leeren.".to_string()
            }
            e => e.to_string(),
        })?;
    if removed > 0 {
        schedule_flush(&app);
    }
    Ok(removed)
}

/// Moves messages to the trash (or deletes them for good if they are in the
/// trash already).
#[tauri::command]
fn delete_messages(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>) -> CmdResult<usize> {
    let deleted = state
        .store
        .lock()
        .map_err(err)?
        .queue_delete(&ids)
        .map_err(|e| match e {
            halcyon_core::Error::Other(m) if m.contains("trash") => {
                "Dieses Konto hat keinen Papierkorb-Ordner.".to_string()
            }
            e => e.to_string(),
        })?;
    if deleted > 0 {
        schedule_flush(&app);
    }
    Ok(deleted)
}

/// Sends queued local changes in the background. Calls while a run is active
/// make it run once more instead of starting a second one.
fn schedule_flush(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.flushing.swap(true, Ordering::SeqCst) {
        state.flush_again.store(true, Ordering::SeqCst);
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            app.state::<AppState>()
                .flush_again
                .store(false, Ordering::SeqCst);
            run_flush(&app).await;
            if !app
                .state::<AppState>()
                .flush_again
                .swap(false, Ordering::SeqCst)
            {
                break;
            }
        }
        app.state::<AppState>()
            .flushing
            .store(false, Ordering::SeqCst);
    });
}

async fn run_flush(app: &AppHandle) {
    let db_path = app.state::<AppState>().db_path.clone();
    let report = |email: &str, error: String| {
        let _ = app.emit(
            "sync",
            SyncUpdate::Failed {
                email: email.to_string(),
                error: format!("Änderung nicht übertragen: {error}"),
            },
        );
    };
    let mut store = match Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => return report("", e.to_string()),
    };
    for account in store.accounts().unwrap_or_default() {
        if store
            .pending_ops(account.id)
            .map(|o| o.is_empty())
            .unwrap_or(true)
        {
            continue;
        }
        let email = account.email.clone();
        let password = tauri::async_runtime::spawn_blocking(move || {
            secrets::load(&email, SecretKind::ImapPassword)
        })
        .await;
        let Ok(Ok(Some(password))) = password else {
            report(&account.email, "kein Passwort im Schlüsselbund".into());
            continue;
        };
        let result = async {
            let (mut session, caps) =
                imap::login(&account.imap, &Credentials::Password(password)).await?;
            let sent = sync::flush_changes(&mut store, &mut session, &caps, account.id).await;
            let _ = session.logout().await;
            sent
        }
        .await;
        if let Err(e) = result {
            report(&account.email, e.to_string());
        }
    }
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
async fn brand_logo(state: State<'_, AppState>, domain: String) -> CmdResult<Option<BrandLogo>> {
    let domain = domain.trim().to_ascii_lowercase();
    if !brand::is_valid_domain(&domain) {
        return Ok(None);
    }

    let now = now_unix_secs();
    {
        let store = state.store.lock().map_err(err)?;
        if let Some(cached) = store.get_brand_logo(&domain, now).map_err(err)? {
            return Ok(match cached {
                CachedLogo::Hit(logo) => Some(logo),
                CachedLogo::Miss => None,
            });
        }
    }

    let _permit = state.logo_semaphore.acquire().await.map_err(err)?;
    // Re-check cache after acquiring semaphore in case another request resolved it.
    {
        let store = state.store.lock().map_err(err)?;
        if let Some(cached) = store
            .get_brand_logo(&domain, now_unix_secs())
            .map_err(err)?
        {
            return Ok(match cached {
                CachedLogo::Hit(logo) => Some(logo),
                CachedLogo::Miss => None,
            });
        }
    }

    let resolved = brand::resolve_logo(&domain)
        .await
        .map(|data_url| BrandLogo {
            data_url,
            bg: None,
            padded: true,
            scale: 1.0,
        });

    {
        let store = state.store.lock().map_err(err)?;
        store
            .save_brand_logo(&domain, resolved.as_ref(), now_unix_secs())
            .map_err(err)?;
    }

    Ok(resolved)
}

#[tauri::command]
fn save_brand_logo_appearance(
    state: State<'_, AppState>,
    domain: String,
    bg: Option<String>,
    padded: bool,
    scale: f32,
) -> CmdResult<()> {
    let domain = domain.trim().to_ascii_lowercase();
    if domain.is_empty() {
        return Ok(());
    }
    let now = now_unix_secs();
    let store = state.store.lock().map_err(err)?;
    if let Some(CachedLogo::Hit(mut logo)) = store.get_brand_logo(&domain, now).map_err(err)? {
        logo.bg = bg;
        logo.padded = padded;
        logo.scale = scale;
        store
            .save_brand_logo(&domain, Some(&logo), now)
            .map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
fn clear_brand_logos(state: State<'_, AppState>) -> CmdResult<()> {
    state
        .store
        .lock()
        .map_err(err)?
        .clear_brand_logos()
        .map_err(err)
}

// ----- cases ("Vorgänge") ---------------------------------------------------

/// Files new mail into cases in the background and tells the UI when cases
/// changed. Calls while a run is active lead to one more run afterwards.
pub(crate) fn schedule_link(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.linking.swap(true, Ordering::SeqCst) {
        state.link_again.store(true, Ordering::SeqCst);
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        loop {
            state.link_again.store(false, Ordering::SeqCst);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let result = match state.store.lock() {
                Ok(mut store) => linker::run(&mut store, now),
                Err(_) => break,
            };
            match result {
                Ok(report) if report.changed() => {
                    tracing::info!(?report, "cases updated");
                    let _ = app.emit("cases-changed", ());
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "case linking failed"),
            }
            if !state.link_again.load(Ordering::SeqCst) {
                break;
            }
        }
        state.linking.store(false, Ordering::SeqCst);
    });
}

#[tauri::command]
fn cases(state: State<'_, AppState>) -> CmdResult<Vec<CaseView>> {
    state
        .store
        .lock()
        .map_err(err)?
        .list_cases(false)
        .map_err(err)
}

/// Sets a case's status from the card ("Erledigt", "Kein Vorgang").
#[tauri::command]
fn set_case_status(state: State<'_, AppState>, id: i64, status: String) -> CmdResult<()> {
    let status: CaseStatus = status.parse().map_err(err)?;
    state
        .store
        .lock()
        .map_err(err)?
        .set_case_status(id, status)
        .map_err(err)
}

/// The mail of a case, wherever it is stored.
#[tauri::command]
fn messages_by_ids(state: State<'_, AppState>, ids: Vec<i64>) -> CmdResult<Vec<MessageSummary>> {
    let store = state.store.lock().map_err(err)?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids.into_iter().take(2_000) {
        if let Some(m) = store.message(id).map_err(err)? {
            out.push(m);
        }
    }
    Ok(out)
}

/// One-time data jobs after an update, in the background so the window
/// opens at once. Currently: DMARC verdicts (for sender logos) from stored
/// originals of mail synced before they were recorded.
fn backfill_once(app: &AppHandle) {
    let app = app.clone();
    let db_path = app.state::<AppState>().db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Ok(store) = Store::open(&db_path) else {
            return;
        };
        match store.backfill_dmarc_from_bodies() {
            Ok(Some(n)) if n > 0 => {
                tracing::info!(authenticated = n, "DMARC verdicts backfilled");
                // Any account id: the UI reloads list and counts.
                let _ = app.emit(
                    "mail-changed",
                    watch::MailChanged {
                        account_id: 0,
                        new_messages: 0,
                    },
                );
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "DMARC backfill failed"),
        }
    });
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
            flushing: AtomicBool::new(false),
            flush_again: AtomicBool::new(false),
            watchers: watch::Watchers::default(),
            outbox: outbox::Outbox::default(),
            logo_semaphore: tokio::sync::Semaphore::new(4),
            linking: AtomicBool::new(false),
            link_again: AtomicBool::new(false),
            verified_servers: Mutex::new(HashMap::new()),
        })
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // Live mail from the start: watchers per account and one full
            // sync to catch up on what arrived while the app was closed.
            watch::restart(app.handle());
            outbox::start(app.handle());
            backfill_once(app.handle());
            schedule_link(app.handle());
            begin_sync(app.handle());
            Ok(())
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
            delete_account,
            message_content,
            open_link,
            save_attachment,
            set_flag,
            delete_messages,
            compose_reply,
            mark_folders_read,
            empty_folder,
            send_message,
            cancel_send,
            take_failed,
            outbox_list,
            brand_logo,
            save_brand_logo_appearance,
            clear_brand_logos,
            cases,
            set_case_status,
            messages_by_ids
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Halcyon app");
}
