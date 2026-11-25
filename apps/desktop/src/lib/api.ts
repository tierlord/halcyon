// Single entry point to the backend. Outside Tauri (plain `pnpm dev` in a
// browser) it serves mock data, so the UI can be built without accounts.

import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  Account,
  DiscoverResult,
  Draft,
  OutboxEvent,
  Recipient,
  ReplyKind,
  LiveState,
  MaskedPreview,
  MessageTurnInfo,
  MessageView,
  MessageSummary,
  Overview,
  ServerConfig,
  SourceMode,
  SourceRecord,
  SyncUpdate,
  View,
} from './types'
import * as mock from './mock'
import * as bundlesMock from './bundlesMock'
import type {
  Assignment,
  AssignmentPreview,
  Bundle,
  BundlePatch,
  Placement,
  Target,
} from './bundles'
import { createMockCases, fromCaseView, type AsideCategory, type CaseItem, type CaseStatus, type CaseView } from './cases'

const native = isTauri()

export const isMock = !native

export function overview(): Promise<Overview> {
  return native ? invoke('overview') : mock.overview()
}

export function messages(view: View): Promise<MessageSummary[]> {
  const accountId = view.kind === 'unified' ? null : view.accountId
  const role = view.kind === 'folder' ? null : view.role
  const folderId = view.kind === 'folder' ? view.folderId : null
  return native
    ? invoke('messages', { accountId, role, folderId })
    : mock.messages(accountId, role, folderId)
}

export function search(query: string): Promise<MessageSummary[]> {
  return native ? invoke('search', { query }) : mock.search(query)
}

/** Starts a background sync; resolves false if one is already running. */
export function startSync(): Promise<boolean> {
  return native ? invoke('start_sync') : mock.startSync()
}

export async function onSync(handler: (u: SyncUpdate) => void): Promise<() => void> {
  if (!native) return mock.onSync(handler)
  return listen<SyncUpdate>('sync', (e) => handler(e.payload))
}

/** Live watchers: connected with push, polling, or offline. */
export async function onLive(handler: (s: LiveState) => void): Promise<() => void> {
  if (!native) return () => {}
  return listen<LiveState>('live', (e) => handler(e.payload))
}

/** A background watcher started or finished syncing an account. */
export async function onActivity(
  handler: (a: { email: string; active: boolean }) => void,
): Promise<() => void> {
  if (!native) return () => {}
  return listen<{ email: string; active: boolean }>('activity', (e) => handler(e.payload))
}

/** An inbox changed in the background (new, removed or re-flagged mail). */
export async function onMailChanged(
  handler: (c: { account_id: number; new_messages: number }) => void,
): Promise<() => void> {
  if (!native) return () => {}
  return listen<{ account_id: number; new_messages: number }>('mail-changed', (e) =>
    handler(e.payload),
  )
}

export function listAccounts(): Promise<Account[]> {
  return native ? invoke('list_accounts') : mock.listAccounts()
}

export function discoverAccount(email: string): Promise<DiscoverResult> {
  return native ? invoke('discover_account', { email }) : mock.discoverAccount(email)
}

/** Checks reachability and TLS of a server without sending credentials. */
export function checkServer(email: string, server: ServerConfig): Promise<string> {
  return native ? invoke('check_server', { email, server }) : mock.checkServer(server)
}

export function saveAccount(account: Account, password: string | null): Promise<Account> {
  return native ? invoke('save_account', { account, password }) : mock.saveAccount(account)
}

export function deleteAccount(id: number): Promise<void> {
  return native ? invoke('delete_account', { id }) : mock.deleteAccount(id)
}

/** Message content as a sandbox-ready document. */
export function messageContent(
  id: number,
  allowRemote: boolean,
  dark: boolean,
): Promise<MessageView> {
  return native
    ? invoke('message_content', { id, allowRemote, dark })
    : mock.messageContent(id, allowRemote, dark)
}

/** Remembers a sender so remote images load automatically in future emails. */
export function trustRemoteSender(sender: string): Promise<void> {
  return native ? invoke('trust_remote_sender', { sender }) : Promise.resolve()
}

/** Opens a web or mail link outside the app. */
export function openLink(url: string): Promise<void> {
  return native ? invoke('open_link', { url }) : mock.openLink(url)
}

/** Saves an attachment to the downloads folder; resolves with its path. */
export function saveAttachment(id: number, part: number): Promise<string> {
  return native ? invoke('save_attachment', { id, part }) : mock.saveAttachment(id, part)
}

export function previewImageAttachment(id: number, part: number): Promise<string> {
  if (!native) {
    // 1x1 transparent pixel fallback in mock mode
    return Promise.resolve(
      'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=',
    )
  }
  return invoke('preview_image_attachment', { id, part })
}

/** Marks messages read/unread or flagged; synced to the server later. */
export function setFlag(ids: number[], flag: 'seen' | 'flagged', on: boolean): Promise<number> {
  return native ? invoke('set_flag', { ids, flag, on }) : mock.setFlag(ids, flag, on)
}

/** Resolves a brand logo data URI (and cached appearance) for a registrable domain. */
export function brandLogo(domain: string): Promise<import('./types').BrandLogo | null> {
  return native ? invoke('brand_logo', { domain }) : mock.brandLogo(domain)
}

/** Persists canvas-analyzed background, padding, and scale for a cached brand logo. */
export function saveBrandLogoAppearance(
  domain: string,
  bg: string | null,
  padded: boolean,
  scale: number,
): Promise<void> {
  return native
    ? invoke('save_brand_logo_appearance', { domain, bg, padded, scale })
    : mock.saveBrandLogoAppearance(domain, bg, padded, scale)
}

/** Clears all cached brand logos. */
export function clearBrandLogos(): Promise<void> {
  return native ? invoke('clear_brand_logos') : mock.clearBrandLogos()
}

/** Moves messages to the trash (or deletes them for good from the trash). */
export function deleteMessages(ids: number[]): Promise<number> {
  return native ? invoke('delete_messages', { ids }) : mock.deleteMessages(ids)
}

/** Moves messages to the account's spam folder; rejects if the account has none. */
export function moveToJunk(ids: number[]): Promise<number> {
  return native ? invoke('move_to_junk', { ids }) : mock.moveToJunk(ids)
}

/** "Ich kenne den Absender": a local note on a flagged mail, never sent anywhere. */
export function acknowledgePhishing(messageId: number, on: boolean): Promise<void> {
  return native ? invoke('acknowledge_phishing', { messageId, on }) : mock.acknowledgePhishing(messageId, on)
}

/** Pre-filled reply, reply-all or forward for a message. */
export function composeReply(
  id: number,
  kind: ReplyKind,
): Promise<{
  account_id: number
  draft: { to: Recipient[]; cc: Recipient[]; subject: string; in_reply_to: string | null; references: string[]; body: string }
}> {
  const utcOffsetMinutes = -new Date().getTimezoneOffset()
  return native
    ? invoke('compose_reply', { id, kind, utcOffsetMinutes })
    : mock.composeReply(id, kind)
}

/** Queues a message; it leaves after the undo window. */
export function sendMessage(draft: Draft): Promise<{ id: number; send_after: number }> {
  return native ? invoke('send_message', { draft }) : mock.sendMessage(draft)
}

/** Takes a waiting message back; null if it already left. */
export function cancelSend(id: number): Promise<Draft | null> {
  return native ? invoke('cancel_send', { id }) : mock.cancelSend(id)
}

/** A failed message back to the editor. */
export function takeFailed(id: number): Promise<Draft | null> {
  return native ? invoke('take_failed', { id }) : mock.takeFailed(id)
}

export async function onOutbox(handler: (e: OutboxEvent) => void): Promise<() => void> {
  if (!native) return mock.onOutbox(handler)
  return listen<OutboxEvent>('outbox', (e) => handler(e.payload))
}

/** Marks all messages of these folders read (one server command each). */
export function markFoldersRead(folderIds: number[]): Promise<number> {
  return native ? invoke('mark_folders_read', { folderIds }) : mock.markFoldersRead(folderIds)
}

/** Empties a spam or trash folder for good. */
export function emptyFolder(folderId: number): Promise<number> {
  return native ? invoke('empty_folder', { folderId }) : mock.emptyFolder(folderId)
}

/** Cases ("Vorgänge") and recurring notice groups. Mock mode serves sample cases. */
export async function cases(): Promise<{ cases: CaseItem[]; categories: AsideCategory[] }> {
  if (!native) return createMockCases(mock.allMockMessages())
  const views = await invoke<CaseView[]>('cases')
  return { cases: views.map(fromCaseView), categories: [] }
}

export function setCaseStatus(id: number, status: CaseStatus): Promise<void> {
  return native ? invoke('set_case_status', { id, status }) : Promise.resolve()
}

export function sources(): Promise<SourceRecord[]> {
  return native ? invoke('sources') : Promise.resolve([])
}

export function setSourceMode(id: number, mode: SourceMode): Promise<void> {
  return native ? invoke('set_source_mode', { id, mode }) : Promise.resolve()
}

export function dismissSourceAnomaly(id: number): Promise<void> {
  return native ? invoke('dismiss_source_anomaly', { id }) : Promise.resolve()
}

/** Removes a recurring source and moves its messages to trash (asJunk=false) or junk (asJunk=true). */
export function removeSource(sourceId: number, asJunk: boolean): Promise<number> {
  return native ? invoke<number>('remove_source', { sourceId, asJunk }) : Promise.resolve(0)
}

export function threadTurns(messageIds: number[]): Promise<MessageTurnInfo[]> {
  return native ? invoke('thread_turns', { messageIds }) : Promise.resolve([])
}

// Mock browser mode: dismissed mail ids, so the undo flow can be tried.
const mockDismissedTurns = new Set<number>()

/** Hides the conversation of this mail until a new mail arrives in the thread. */
export function dismissTurn(messageId: number): Promise<void> {
  if (!native) {
    mockDismissedTurns.add(messageId)
    return Promise.resolve()
  }
  return invoke('dismiss_turn', { messageId })
}

/** Mock mode only: applies dismissals to freshly generated sample turns. */
export function applyMockTurnDismissal(t: MessageTurnInfo): MessageTurnInfo {
  return mockDismissedTurns.has(t.message_id)
    ? { ...t, status: 'done', follow_up_due: false, reason: 'Von dir als erledigt entfernt' }
    : t
}

export function restoreTurn(messageId: number): Promise<void> {
  if (!native) {
    mockDismissedTurns.delete(messageId)
    return Promise.resolve()
  }
  return invoke('restore_turn', { messageId })
}

export function maskMessagePreview(id: number): Promise<MaskedPreview> {
  if (!native) {
    return Promise.resolve({
      allowed: true,
      confidential_reason: null,
      masked_subject: '[SUBJECT]',
      masked_body: '[MASKED_BODY]',
      replacements: 0,
    })
  }
  return invoke('mask_message_preview', { id })
}

/** Inbox mail dated `sinceSec` (unix seconds) or later, newest first, up to 3000. */
export function inboxSince(sinceSec: number): Promise<MessageSummary[]> {
  // The browser preview's sample data is already complete (see `allMockMessages`).
  return native ? invoke('inbox_since', { since: sinceSec }) : Promise.resolve([])
}

/** Messages by id, wherever they are stored (the mail of a case). */
export function messagesByIds(ids: number[]): Promise<MessageSummary[]> {
  if (!native) {
    const set = new Set(ids)
    return Promise.resolve(mock.allMockMessages().filter((m) => set.has(m.id)))
  }
  return invoke('messages_by_ids', { ids })
}

/** The linker filed new mail into cases. */
export async function onCasesChanged(handler: () => void): Promise<() => void> {
  if (!native) return () => {}
  return listen('cases-changed', () => handler())
}

/** Your own sent replies to these Message-IDs (to show whole threads). */
export function sentReplies(messageIds: string[]): Promise<MessageSummary[]> {
  if (!native) {
    const set = new Set(messageIds)
    return Promise.resolve(
      mock
        .allMockMessages()
        .filter((m) => m.folder_role === 'sent' && m.header.in_reply_to != null && set.has(m.header.in_reply_to)),
    )
  }
  return invoke('sent_replies', { messageIds })
}

const MOCK_AUTO_READ_KEY = 'halcyon.mock.autoMarkRead'

/** Whether Halcyon automatically marks processed/filed mails as read on the server. */
export function autoMarkRead(): Promise<boolean> {
  if (!native) {
    try {
      return Promise.resolve(localStorage.getItem(MOCK_AUTO_READ_KEY) !== '0')
    } catch {
      return Promise.resolve(true)
    }
  }
  return invoke('auto_mark_read')
}

export function setAutoMarkRead(enabled: boolean): Promise<void> {
  if (!native) {
    try {
      localStorage.setItem(MOCK_AUTO_READ_KEY, enabled ? '1' : '0')
    } catch {
      // Ignore storage errors in restricted contexts.
    }
    return Promise.resolve()
  }
  return invoke('set_auto_mark_read', { enabled })
}

export type InteractionAction = 'open' | 'reply' | 'delete' | 'ignore'

/** Records a user interaction (open, reply, delete, ignore) for sender learning. */
export function recordInteraction(messageId: number, action: InteractionAction): Promise<void> {
  if (!native) return Promise.resolve()
  return invoke('record_interaction', { messageId, action })
}

// ----- bundles and assignments ("Wohin gehört das?") -----------------------------

/** Where each mail belongs; one call for all mail shown on "Heute". */
export function placements(messageIds: number[]): Promise<Placement[]> {
  if (messageIds.length === 0) return Promise.resolve([])
  return native ? invoke('placements', { messageIds }) : bundlesMock.placements(messageIds)
}

/** All mail of the sender grouped by kind: what "alle dieser Art" would take along. */
export function assignmentPreview(messageId: number, target: Target): Promise<AssignmentPreview> {
  return native
    ? invoke('assignment_preview', { messageId, target })
    : bundlesMock.assignmentPreview(messageId, target)
}

/** Without `kindKeys` for this mail alone; returns the ids for "Rückgängig". */
export function applyAssignment(
  messageId: number,
  target: Target,
  kindKeys: string[],
  excludedMessageIds: number[],
): Promise<number[]> {
  return native
    ? invoke('apply_assignment', { messageId, target, kindKeys, excludedMessageIds })
    : bundlesMock.applyAssignment(messageId, target, kindKeys, excludedMessageIds)
}

export function removeAssignment(id: number): Promise<void> {
  return native ? invoke('remove_assignment', { id }) : bundlesMock.removeAssignment(id)
}

export function assignments(): Promise<Assignment[]> {
  return native ? invoke('assignments') : bundlesMock.assignments()
}

/** All bundles in display order, `other` last. */
export function bundles(): Promise<Bundle[]> {
  return native ? invoke('bundles') : bundlesMock.bundles()
}

export function createBundle(name: string): Promise<Bundle> {
  return native ? invoke('create_bundle', { name }) : bundlesMock.createBundle(name)
}

export function updateBundle(key: string, patch: BundlePatch): Promise<Bundle> {
  return native
    ? invoke('update_bundle', {
        key,
        name: patch.name ?? null,
        enabled: patch.enabled ?? null,
        presentation: patch.presentation ?? null,
        markRead: patch.markRead ?? null,
        position: patch.position ?? null,
      })
    : bundlesMock.updateBundle(key, patch)
}

/** Custom bundles only; what the user put into it moves to "Weitere Mails". */
export function deleteBundle(key: string): Promise<void> {
  return native ? invoke('delete_bundle', { key }) : bundlesMock.deleteBundle(key)
}
