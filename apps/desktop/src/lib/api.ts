// Single entry point to the backend. Outside Tauri (plain `pnpm dev` in a
// browser) it serves mock data, so the UI can be built without accounts.

import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { MessageSummary, Overview, SyncUpdate, View } from './types'
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
