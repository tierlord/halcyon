// Single entry point to the backend. Outside Tauri (plain `pnpm dev` in a
// browser) it serves mock data, so the UI can be built without accounts.

import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  Account,
  DiscoverResult,
  MessageView,
  MessageSummary,
  Overview,
  ServerConfig,
  SyncUpdate,
  View,
} from './types'
import * as mock from './mock'

const native = isTauri()

export const isMock = !native

export function overview(): Promise<Overview> {
  return native ? invoke('overview') : mock.overview()
}

export function messages(view: View): Promise<MessageSummary[]> {
  const accountId = view.kind === 'account' ? view.accountId : null
  return native
    ? invoke('messages', { accountId, role: view.role })
    : mock.messages(accountId, view.role)
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
export function messageContent(id: number, allowRemote: boolean): Promise<MessageView> {
  return native
    ? invoke('message_content', { id, allowRemote })
    : mock.messageContent(id, allowRemote)
}

/** Opens a web or mail link outside the app. */
export function openLink(url: string): Promise<void> {
  return native ? invoke('open_link', { url }) : mock.openLink(url)
}

/** Saves an attachment to the downloads folder; resolves with its path. */
export function saveAttachment(id: number, part: number): Promise<string> {
  return native ? invoke('save_attachment', { id, part }) : mock.saveAttachment(id, part)
}

/** Marks messages read/unread or flagged; synced to the server later. */
export function setFlag(ids: number[], flag: 'seen' | 'flagged', on: boolean): Promise<number> {
  return native ? invoke('set_flag', { ids, flag, on }) : mock.setFlag(ids, flag, on)
}

/** Moves messages to the trash (or deletes them for good from the trash). */
export function deleteMessages(ids: number[]): Promise<number> {
  return native ? invoke('delete_messages', { ids }) : mock.deleteMessages(ids)
}
