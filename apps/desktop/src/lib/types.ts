// Mirrors the serde types of halcyon-core and the Tauri commands.

export interface MessageHeader {
  uid: number
  message_id: string | null
  in_reply_to: string | null
  references: string[]
  subject: string | null
  from_name: string | null
  from_addr: string | null
  reply_to: string | null
  to: string[]
  cc: string[]
  /** Unix timestamp in seconds. */
  date: number | null
  size: number | null
  flags: string[]
  list_id: string | null
  list_unsubscribe: string[]
  list_unsubscribe_post: boolean
}

export interface MessageSummary {
  id: number
  account_id: number
  account_email: string
  folder_id: number
  folder_path: string
  folder_name: string
  folder_role: FolderRole | null
  preview: string | null
  has_attachments: boolean
  has_body: boolean
  header: MessageHeader
}

export interface AccountInfo {
  id: number
  email: string
  display_name: string | null
  unread_inbox: number
}

export type Security = 'tls' | 'starttls' | 'plain'
export type AuthMethod = 'password' | 'oauth2'

export interface ServerConfig {
  host: string
  port: number
  security: Security
  username: string
  auth: AuthMethod
}

export interface Account {
  id: number
  email: string
  display_name: string | null
  imap: ServerConfig
  smtp: ServerConfig | null
}

export type DiscoverySource = 'provider' | 'ispdb' | 'mx-provider' | 'mx-ispdb' | 'srv' | 'guess'

export interface DiscoveryCandidate {
  source: DiscoverySource
  provider_name: string | null
  imap: ServerConfig
  smtp: ServerConfig | null
}

export interface DiscoverResult {
  candidates: DiscoveryCandidate[]
  reachable_index: number | null
  probe_error: string | null
}

export interface Overview {
  accounts: AccountInfo[]
  unread_inbox: number
}

export type FolderRole =
  | 'inbox'
  | 'sent'
  | 'drafts'
  | 'trash'
  | 'junk'
  | 'archive'
  | 'all'
  | 'flagged'

/** What the message list shows. */
export type View =
  | { kind: 'unified'; role: FolderRole }
  | { kind: 'account'; accountId: number; role: FolderRole }

export type SyncUpdate =
  | { kind: 'started' }
  | { kind: 'account'; email: string }
  | { kind: 'folder'; email: string; path: string; done: number; total: number }
  | { kind: 'bodies'; email: string; path: string; done: number; total: number }
  | { kind: 'failed'; email: string; error: string }
  | { kind: 'finished'; new_messages: number }

export interface Attachment {
  part: number
  name: string
  mime_type: string
  size: number
  inline: boolean
}

export interface MessageView {
  rendered: { document: string; remote_blocked: number; is_html: boolean; dark: boolean }
  attachments: Attachment[]
}

export type LiveState =
  | { state: 'push'; email: string }
  | { state: 'polling'; email: string }
  | { state: 'offline'; email: string; error: string }

export const isSeen = (m: MessageSummary) => m.header.flags.includes('\\Seen')
export const isFlagged = (m: MessageSummary) => m.header.flags.includes('\\Flagged')
