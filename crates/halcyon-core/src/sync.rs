//! Pulls folders and message headers from the server into the local store.
//!
//! Two-way: local changes (read, flagged, deleted) are queued in the store
//! and sent first; then each folder brings in new messages, removes messages
//! deleted or moved elsewhere, and takes over flag changes made on other
//! devices – with CONDSTORE only the changed ones, otherwise all flags.

use futures::TryStreamExt;

use crate::Result;
use crate::imap::{self, Capabilities, Credentials, HEADER_FIELDS, ImapSession};
use crate::model::{Account, FolderRole, PendingChange};
use crate::store::Store;

/// Messages fetched per round trip.
const FETCH_CHUNK: usize = 250;

/// Content of messages from this many days is downloaded during sync;
/// older messages are fetched when opened.
const PREFETCH_DAYS: i64 = 90;
/// Larger messages (usually because of attachments) are fetched on demand.
const PREFETCH_MAX_SIZE: u32 = 2 * 1024 * 1024;
/// Upper bounds for one round trip of content downloads.
const BODY_BATCH_BYTES: u64 = 8 * 1024 * 1024;
const BODY_BATCH_MESSAGES: usize = 50;

#[derive(Debug)]
pub enum SyncEvent<'a> {
    FolderStarted {
        path: &'a str,
        new_messages: usize,
    },
    Progress {
        path: &'a str,
        done: usize,
        total: usize,
    },
    /// The server changed UIDVALIDITY, so the folder is downloaded again.
    FolderReset {
        path: &'a str,
    },
    FolderFailed {
        path: &'a str,
        error: &'a crate::Error,
    },
    /// Message content downloaded in the background.
    Bodies {
        path: &'a str,
        done: usize,
        total: usize,
    },
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub folders: usize,
    pub new_messages: usize,
    pub bodies: usize,
    /// Local changes sent to the server.
    pub changes_sent: usize,
    /// Messages removed because they are gone from the server.
    pub removed: usize,
    /// Messages whose flags changed on the server.
    pub flags_updated: usize,
    pub failed_folders: Vec<(String, String)>,
}

/// Syncs one account. Errors in single folders are reported through
/// `on_event` and the report; only connection and login errors abort.
pub async fn sync_account(
    store: &mut Store,
    account: &Account,
    credentials: &Credentials,
    mut on_event: impl FnMut(SyncEvent<'_>),
) -> Result<SyncReport> {
    let (mut session, caps) = imap::login(&account.imap, credentials).await?;
    tracing::debug!(account = %account.email, ?caps, "logged in");

    // Local changes first, so the flag sync below cannot undo them.
    let changes_sent = flush_changes(store, &mut session, &caps, account.id).await?;

    let remote = imap::list_folders(&mut session).await?;
    let paths: Vec<String> = remote.iter().map(|f| f.path.clone()).collect();
    store.retain_folders(account.id, &paths)?;

    let mut report = SyncReport {
        changes_sent,
        ..SyncReport::default()
    };
    for rf in &remote {
        let folder = store.upsert_folder(
            account.id,
            &rf.path,
            rf.delimiter.as_deref(),
            rf.role,
            rf.selectable,
        )?;
        // Virtual folders (Gmail's "All Mail", "Starred") only duplicate
        // messages that already live in real folders.
        if !folder.selectable || matches!(folder.role, Some(FolderRole::All | FolderRole::Flagged))
        {
            continue;
        }
        report.folders += 1;
        let skip_bodies = matches!(folder.role, Some(FolderRole::Trash));
        match sync_folder(
            store,
            &mut session,
            &caps,
            folder.id,
            &rf.path,
            skip_bodies,
            &mut on_event,
        )
        .await
        {
            Ok(r) => {
                report.new_messages += r.new_messages;
                report.bodies += r.bodies;
                report.removed += r.removed;
                report.flags_updated += r.flags_updated;
            }
            Err(e) => {
                on_event(SyncEvent::FolderFailed {
                    path: &rf.path,
                    error: &e,
                });
                report.failed_folders.push((rf.path.clone(), e.to_string()));
                if matches!(
                    e,
                    crate::Error::Imap(async_imap::error::Error::ConnectionLost)
                ) {
                    break;
                }
            }
        }
    }

    let _ = session.logout().await;
    Ok(report)
}

#[derive(Default)]
struct FolderResult {
    new_messages: usize,
    bodies: usize,
    removed: usize,
    flags_updated: usize,
}

async fn sync_folder(
    store: &mut Store,
    session: &mut ImapSession,
    caps: &Capabilities,
    folder_id: i64,
    path: &str,
    skip_bodies: bool,
    on_event: &mut impl FnMut(SyncEvent<'_>),
) -> Result<FolderResult> {
    let mailbox = if caps.condstore {
        session.select_condstore(path).await?
    } else {
        session.examine(path).await?
    };
    // Servers without UIDVALIDITY violate RFC 3501; treat them as constant.
    if store.set_uid_validity(folder_id, mailbox.uid_validity.unwrap_or(0))? {
        on_event(SyncEvent::FolderReset { path });
    }
    let folder = store
        .folder_by_id(folder_id)?
        .ok_or_else(|| crate::Error::Other(format!("folder {path} vanished")))?;
    let last_seen = folder.last_seen_uid;
    let mut result = FolderResult::default();

    // Changes to messages we already have: deletions and flags.
    if last_seen > 0 {
        let existing: std::collections::HashSet<u32> = if mailbox.exists == 0 {
            Default::default()
        } else {
            session.uid_search("ALL").await?.into_iter().collect()
        };
        result.removed = store.delete_missing(folder_id, &existing, last_seen)?;

        let unchanged = caps.condstore
            && folder.highest_modseq.is_some()
            && folder.highest_modseq == mailbox.highest_modseq;
        if mailbox.exists > 0 && !unchanged {
            let query = match (caps.condstore, folder.highest_modseq) {
                (true, Some(modseq)) => format!("(UID FLAGS) (CHANGEDSINCE {modseq})"),
                _ => "(UID FLAGS)".to_string(),
            };
            let fetches: Vec<_> = session
                .uid_fetch(format!("1:{last_seen}"), &query)
                .await?
                .try_collect()
                .await?;
            let flags: Vec<(u32, Vec<String>)> = fetches
                .iter()
                .filter_map(|f| Some((f.uid?, imap::flag_names(f))))
                .filter(|(uid, _)| *uid <= last_seen)
                .collect();
            result.flags_updated = store.apply_server_flags(folder_id, &flags)?;
        }
    }
    if mailbox.exists == 0 {
        on_event(SyncEvent::FolderStarted {
            path,
            new_messages: 0,
        });
        store.set_highest_modseq(folder_id, mailbox.highest_modseq)?;
        return Ok(result);
    }
    // "n:*" always matches the highest UID, even if it is below n.
    let mut uids: Vec<u32> = session
        .uid_search(format!("UID {}:*", last_seen + 1))
        .await?
        .into_iter()
        .filter(|&uid| uid > last_seen)
        .collect();
    // Newest first, so the user sees recent mail while older mail loads.
    uids.sort_unstable_by(|a, b| b.cmp(a));
    on_event(SyncEvent::FolderStarted {
        path,
        new_messages: uids.len(),
    });

    let query =
        format!("(UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER.FIELDS ({HEADER_FIELDS})])");
    let mut done = 0;
    for chunk in uids.chunks(FETCH_CHUNK) {
        let fetches: Vec<_> = session
            .uid_fetch(uid_set(chunk), &query)
            .await?
            .try_collect()
            .await?;
        let headers: Vec<_> = fetches.iter().filter_map(imap::header_from_fetch).collect();
        store.insert_messages(folder_id, &headers)?;
        done += chunk.len();
        on_event(SyncEvent::Progress {
            path,
            done,
            total: uids.len(),
        });
    }
    if let Some(&max) = uids.first() {
        store.mark_synced_up_to(folder_id, max)?;
    }
    // Only now: if anything above failed, the next sync asks for the same
    // changes again.
    store.set_highest_modseq(folder_id, mailbox.highest_modseq)?;
    result.new_messages = uids.len();

    if !skip_bodies {
        result.bodies = prefetch_bodies(store, session, folder_id, path, on_event).await?;
    }
    Ok(result)
}

/// Sends queued local changes of one account. Changes that fail are kept
/// and retried on later syncs (a few times). Returns how many were sent.
pub async fn flush_changes(
    // `&mut` although only reading and writing rows: it keeps the future
    // `Send` (a shared reference would need the connection to be `Sync`).
    store: &mut Store,
    session: &mut ImapSession,
    caps: &Capabilities,
    account_id: i64,
) -> Result<usize> {
    let ops = store.pending_ops(account_id)?;
    let mut sent = 0;
    let mut selected: Option<String> = None;
    for op in ops {
        if selected.as_deref() != Some(op.folder_path.as_str()) {
            session.select(&op.folder_path).await?;
            selected = Some(op.folder_path.clone());
        }
        match apply_change(session, caps, op.uid, &op.change).await {
            Ok(()) => {
                store.complete_op(op.id)?;
                sent += 1;
            }
            Err(e) => {
                tracing::warn!(op = op.id, error = %e, "could not send change");
                store.fail_op(op.id, &e.to_string())?;
                // A broken connection fails every following op too.
                if matches!(
                    e,
                    crate::Error::Imap(async_imap::error::Error::ConnectionLost)
                ) {
                    return Err(e);
                }
            }
        }
    }
    Ok(sent)
}

async fn apply_change(
    session: &mut ImapSession,
    caps: &Capabilities,
    uid: u32,
    change: &PendingChange,
) -> Result<()> {
    let uid = uid.to_string();
    match change {
        PendingChange::AddFlag(flag) => {
            store_flags(session, &uid, &format!("+FLAGS.SILENT ({flag})")).await
        }
        PendingChange::RemoveFlag(flag) => {
            store_flags(session, &uid, &format!("-FLAGS.SILENT ({flag})")).await
        }
        PendingChange::MoveTo(target) => {
            if caps.r#move {
                session.uid_mv(&uid, target).await?;
                Ok(())
            } else {
                session.uid_copy(&uid, target).await?;
                store_flags(session, &uid, "+FLAGS.SILENT (\\Deleted)").await?;
                expunge(session, caps, &uid).await
            }
        }
        PendingChange::Expunge => {
            store_flags(session, &uid, "+FLAGS.SILENT (\\Deleted)").await?;
            expunge(session, caps, &uid).await
        }
    }
}

async fn store_flags(session: &mut ImapSession, uid: &str, query: &str) -> Result<()> {
    let _: Vec<_> = session.uid_store(uid, query).await?.try_collect().await?;
    Ok(())
}

/// With UIDPLUS only this message is expunged; without it EXPUNGE removes
/// every message marked deleted in the folder, as other clients expect.
async fn expunge(session: &mut ImapSession, caps: &Capabilities, uid: &str) -> Result<()> {
    if caps.uidplus {
        let _: Vec<_> = session.uid_expunge(uid).await?.try_collect().await?;
    } else {
        let _: Vec<_> = session.expunge().await?.try_collect().await?;
    }
    Ok(())
}

/// Downloads the content of recent messages that do not have it yet.
async fn prefetch_bodies(
    store: &mut Store,
    session: &mut ImapSession,
    folder_id: i64,
    path: &str,
    on_event: &mut impl FnMut(SyncEvent<'_>),
) -> Result<usize> {
    let since = unix_now() - PREFETCH_DAYS * 24 * 3600;
    let missing = store.messages_without_body(folder_id, since, PREFETCH_MAX_SIZE, usize::MAX)?;
    if missing.is_empty() {
        return Ok(0);
    }
    let sizes = store.message_sizes(&missing.iter().map(|(id, _)| *id).collect::<Vec<_>>())?;
    let total = missing.len();
    let mut done = 0;
    for batch in size_batches(&missing, &sizes) {
        let uids: Vec<u32> = batch.iter().map(|(_, uid)| *uid).collect();
        let fetched = imap::fetch_raw(session, &uid_set(&uids)).await?;
        for (uid, raw) in fetched {
            if let Some((id, _)) = batch.iter().find(|(_, u)| *u == uid) {
                store.store_body(*id, &raw)?;
            }
        }
        done += batch.len();
        on_event(SyncEvent::Bodies { path, done, total });
    }
    Ok(done)
}

/// Splits messages into round trips of bounded size and count.
fn size_batches<'a>(
    messages: &'a [(i64, u32)],
    sizes: &std::collections::HashMap<i64, u32>,
) -> Vec<&'a [(i64, u32)]> {
    let mut batches = Vec::new();
    let mut start = 0;
    let mut bytes = 0u64;
    for (i, (id, _)) in messages.iter().enumerate() {
        let size = u64::from(sizes.get(id).copied().unwrap_or(0));
        if i > start && (bytes + size > BODY_BATCH_BYTES || i - start >= BODY_BATCH_MESSAGES) {
            batches.push(&messages[start..i]);
            start = i;
            bytes = 0;
        }
        bytes += size;
    }
    if start < messages.len() {
        batches.push(&messages[start..]);
    }
    batches
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Downloads one complete message (e.g. when an old or large message is
/// opened). Takes no store, so callers can hold their database connection
/// only briefly around it.
pub async fn download_message(
    account: &Account,
    credentials: &Credentials,
    folder_path: &str,
    uid: u32,
) -> Result<Vec<u8>> {
    let (mut session, _) = imap::login(&account.imap, credentials).await?;
    session.examine(folder_path).await?;
    let fetched = imap::fetch_raw(&mut session, &uid.to_string()).await?;
    let _ = session.logout().await;
    fetched
        .into_iter()
        .find(|(u, _)| *u == uid)
        .map(|(_, raw)| raw)
        .ok_or_else(|| crate::Error::Other("the message no longer exists on the server".into()))
}

/// Compresses UIDs into an IMAP sequence set: `[9, 3, 4, 5, 1]` → `1,3:5,9`.
fn uid_set(uids: &[u32]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts = Vec::new();
    let mut iter = sorted.into_iter().peekable();
    while let Some(start) = iter.next() {
        let mut end = start;
        while iter.peek() == Some(&(end + 1)) {
            end = iter.next().unwrap();
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}:{end}")
        });
    }
    parts.join(",")
}

#[cfg(test)]
mod tests {
    use super::uid_set;

    #[test]
    fn uid_sets_are_compressed() {
        assert_eq!(uid_set(&[9, 3, 4, 5, 1]), "1,3:5,9");
        assert_eq!(uid_set(&[7]), "7");
        assert_eq!(uid_set(&[2, 2, 1]), "1:2");
    }

    #[test]
    fn batches_respect_size_and_count() {
        let msgs: Vec<(i64, u32)> = (1..=5).map(|i| (i, i as u32)).collect();
        let mut sizes = std::collections::HashMap::new();
        sizes.insert(1, 5 * 1024 * 1024);
        sizes.insert(2, 5 * 1024 * 1024);
        sizes.insert(3, 100);
        sizes.insert(4, 100);
        sizes.insert(5, 100);
        let b = super::size_batches(&msgs, &sizes);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].len(), 1);
        assert_eq!(b[1].len(), 4);
        assert!(super::size_batches(&[], &sizes).is_empty());
    }
}
