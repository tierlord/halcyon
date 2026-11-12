//! Pulls folders and message headers from the server into the local store.
//!
//! This first version only adds new messages. Flag changes and deletions of
//! already stored messages will follow with CONDSTORE/QRESYNC, falling back
//! to a periodic flag scan on servers without them.

use futures::TryStreamExt;

use crate::Result;
use crate::imap::{self, Credentials, HEADER_FIELDS, ImapSession};
use crate::model::{Account, FolderRole};
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

    let remote = imap::list_folders(&mut session).await?;
    let paths: Vec<String> = remote.iter().map(|f| f.path.clone()).collect();
    store.retain_folders(account.id, &paths)?;

    let mut report = SyncReport::default();
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
            folder.id,
            &rf.path,
            skip_bodies,
            &mut on_event,
        )
        .await
        {
            Ok((n, b)) => {
                report.new_messages += n;
                report.bodies += b;
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

async fn sync_folder(
    store: &mut Store,
    session: &mut ImapSession,
    folder_id: i64,
    path: &str,
    skip_bodies: bool,
    on_event: &mut impl FnMut(SyncEvent<'_>),
) -> Result<(usize, usize)> {
    let mailbox = session.examine(path).await?;
    // Servers without UIDVALIDITY violate RFC 3501; treat them as constant.
    if store.set_uid_validity(folder_id, mailbox.uid_validity.unwrap_or(0))? {
        on_event(SyncEvent::FolderReset { path });
    }
    if mailbox.exists == 0 {
        on_event(SyncEvent::FolderStarted {
            path,
            new_messages: 0,
        });
        return Ok((0, 0));
    }

    let last_seen = store
        .folder_by_id(folder_id)?
        .map(|f| f.last_seen_uid)
        .unwrap_or(0);
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

    let bodies = if skip_bodies {
        0
    } else {
        prefetch_bodies(store, session, folder_id, path, on_event).await?
    };
    Ok((uids.len(), bodies))
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
