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
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub folders: usize,
    pub new_messages: usize,
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
        match sync_folder(store, &mut session, folder.id, &rf.path, &mut on_event).await {
            Ok(n) => report.new_messages += n,
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
    on_event: &mut impl FnMut(SyncEvent<'_>),
) -> Result<usize> {
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
        return Ok(0);
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
    Ok(uids.len())
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
}
