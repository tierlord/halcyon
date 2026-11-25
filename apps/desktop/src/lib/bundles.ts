// Bundles ("Übersichten", "Werbung", ...) and assignments on the UI side:
// types mirroring the serde shapes of halcyon-core (`bundles.rs`,
// `assignments.rs`), small helpers and the pure grouping of mail into the
// sections shown on "Heute". Backend calls live in api.ts, the browser mock
// in bundlesMock.ts.

import { listDate } from './format'
import type { MessageSummary } from './types'

// ----- types (mirror halcyon-core) ---------------------------------------------

/** `{ kind: 'for_you' }` or `{ kind: 'bundle', bundle: '<key>' }`. */
export type Target = { kind: 'for_you' } | { kind: 'bundle'; bundle: string }

export type Presentation = 'shelf' | 'collapsed'

export interface Bundle {
  key: string
  name: string
  builtin: boolean
  position: number
  enabled: boolean
  presentation: Presentation
  mark_read: boolean
}

export type Origin = 'mail' | 'kind' | 'auto' | 'suspicious'

export interface Placement {
  message_id: number
  target: Target | null
  origin: Origin | null
  assigned_at: number | null
  held_back: Target | null
  bundle: string | null
  bundle_name: string | null
  reason: string | null
  kind_key: string
  kind_label: string
  protected: boolean
  protection: string[]
}

export interface ProtectedMail {
  message_id: number
  subject: string
  /** Unix seconds. */
  date: number
  reasons: string[]
}

export interface PreviewGroup {
  kind_key: string
  label: string
  features: string[]
  count: number
  protected_count: number
  examples: string[]
  first_date: number
  last_date: number
  preselected: boolean
  message_ids: number[]
  protected: ProtectedMail[]
  assigned: Target | null
}

export interface AssignmentPreview {
  message_id: number
  sender: string
  sender_name: string | null
  target: Target
  kind_key: string
  groups: PreviewGroup[]
  total_mails: number
  group_count: number
  selected_mails: number
  selected_protected: number
  other_mails: number
  protected_total: number
}

export interface Assignment {
  id: number
  scope: 'mail' | 'kind'
  target: Target
  created_at: number
  rfc_message_id: string | null
  sender: string | null
  kind_key: string | null
  kind_label: string | null
  subject: string | null
  exceptions: number
}

/** What `update_bundle` may change; absent fields stay as they are. */
export interface BundlePatch {
  name?: string
  enabled?: boolean
  presentation?: Presentation
  markRead?: boolean
  position?: number
}

export const OTHER_KEY = 'other'

// ----- targets -------------------------------------------------------------------

export const FOR_YOU: Target = { kind: 'for_you' }

export function bundleTarget(key: string): Target {
  return { kind: 'bundle', bundle: key }
}

/** Stable string form, usable as DOM attribute and map key. */
export function targetKey(t: Target): string {
  return t.kind === 'for_you' ? 'for_you' : `bundle:${t.bundle}`
}

export function parseTargetKey(key: string): Target {
  return key.startsWith('bundle:') ? bundleTarget(key.slice('bundle:'.length)) : FOR_YOU
}

export function sameTarget(a: Target, b: Target): boolean {
  return targetKey(a) === targetKey(b)
}

export function targetName(t: Target, bundles: Bundle[]): string {
  if (t.kind === 'for_you') return 'Für dich'
  return bundles.find((b) => b.key === t.bundle)?.name ?? 'Bündel'
}

/** Bundles in display order: `other` always last. */
export function orderedBundles(bundles: Bundle[]): Bundle[] {
  return [...bundles].sort(
    (a, b) =>
      Number(a.key === OTHER_KEY) - Number(b.key === OTHER_KEY) || a.position - b.position,
  )
}

// ----- wording -------------------------------------------------------------------

const WEEKDAY = new Intl.DateTimeFormat('de-DE', { weekday: 'long' })
const DAY_LONG = new Intl.DateTimeFormat('de-DE', { day: 'numeric', month: 'long' })
const DAY_SHORT = new Intl.DateTimeFormat('de-DE', { day: 'numeric', month: 'short' })

/** "liegt seit Montag bereit". */
export function readySince(dateSec: number, now = new Date()): string {
  const d = new Date(dateSec * 1000)
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
  const days = Math.round(
    (startOfToday - new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()) / 86_400_000,
  )
  if (days <= 0) return 'liegt seit heute bereit'
  if (days === 1) return 'liegt seit gestern bereit'
  if (days < 7) return `liegt seit ${WEEKDAY.format(d)} bereit`
  return `liegt seit dem ${DAY_LONG.format(d)} bereit`
}

export function plural(n: number, one: string, many: string): string {
  return n === 1 ? one : many
}

export function joinWords(parts: string[], last = 'und'): string {
  if (parts.length <= 1) return parts.join('')
  return `${parts.slice(0, -1).join(', ')} ${last} ${parts.at(-1)}`
}

/** "12. Okt. – 24. Nov." (one day when both ends fall on the same day). */
export function dateRange(firstSec: number, lastSec: number): string {
  const a = DAY_SHORT.format(new Date(firstSec * 1000))
  const b = DAY_SHORT.format(new Date(lastSec * 1000))
  return a === b ? a : `${a} – ${b}`
}

/** The reason for a protected mail, as a plural predicate for "weil sie …". */
const REASON_PLURAL: Record<string, string> = {
  'enthält einen Einmalcode': 'einen Einmalcode enthalten',
  'gehört zu einer Bestellung': 'zu Bestellungen gehören',
  'gehört zu einer Rechnung': 'zu Rechnungen gehören',
  'enthält eine Sendungsnummer': 'eine Sendungsnummer enthalten',
  'enthält eine Rechnung': 'eine Rechnung enthalten',
  'Rechnung noch nicht bezahlt': 'zu offenen Rechnungen gehören',
  'du hast darauf geantwortet': 'du darauf geantwortet hast',
  'wartet auf deine Antwort': 'auf deine Antwort warten',
}

export function becauseClause(reasons: string[]): string {
  const distinct = [...new Set(reasons)]
  const parts = distinct.slice(0, 2).map((r) => REASON_PLURAL[r] ?? null)
  if (parts.length === 0 || parts.some((p) => p == null)) return 'weil sie wichtig sein können'
  return `weil sie ${joinWords(parts as string[], 'oder')}`
}

// ----- sections for "Heute" ---------------------------------------------------------

/** One mail as a row of a bundle (sender, subject, time). */
export interface BundleRow {
  message: MessageSummary
  sender: string
  subject: string
  /** Unix seconds. */
  date: number
  brandEmail: string | null
  /** Arrived since the last visit of "Heute": marked quietly, never counted on the row. */
  isNew: boolean
}

/** A shelf tile: the newest mail of one sender, older ones behind it. */
export interface ShelfTile {
  key: string
  name: string
  brandEmail: string | null
  label: string
  /** "liegt seit Montag bereit". */
  since: string
  newest: BundleRow
  older: BundleRow[]
}

export interface BundleSectionModel {
  bundle: Bundle
  /** Newest first. Collapsed bundle: the mail of the last 14 days. For a shelf: every row of every tile. */
  rows: BundleRow[]
  /** Collapsed bundle: also the mail older than the window `rows` is cut at. */
  allRows: BundleRow[]
  /** "3 neu seit gestern · 41 in den letzten 14 Tagen, zuletzt von Beispielshop". */
  sentence: string
  tiles: ShelfTile[]
}

export interface GroupInput {
  bundles: Bundle[]
  placements: Map<number, Placement>
  /** Mail that is not shown elsewhere on "Heute". */
  rows: BundleRow[]
  /** Unix seconds of the last visit: mail since then is marked as new. */
  since: number
  /** "Seit Montag": the wording for that time. */
  sinceLabel: string
  now?: Date
}

function lowerFirst(s: string): string {
  return s.charAt(0).toLowerCase() + s.slice(1)
}

/** Bundle a mail is shown in: its placement, else "Weitere Mails". */
export function bundleOf(p: Placement | undefined, enabled: Set<string>): string {
  const key = p?.bundle ?? (p?.target?.kind === 'bundle' ? p.target.bundle : null)
  return key && enabled.has(key) ? key : OTHER_KEY
}

/** A collapsed bundle lists the mail of this many days, whatever the last visit. */
export const BUNDLE_WINDOW_DAYS = 14

export function groupBundles(input: GroupInput): BundleSectionModel[] {
  const now = input.now ?? new Date()
  const bundles = orderedBundles(input.bundles).filter((b) => b.enabled || b.key === OTHER_KEY)
  const enabled = new Set(bundles.map((b) => b.key))
  const byBundle = new Map<string, BundleRow[]>()
  for (const row of input.rows) {
    const key = bundleOf(input.placements.get(row.message.id), enabled)
    const list = byBundle.get(key) ?? []
    list.push(row)
    byBundle.set(key, list)
  }
  const sections: BundleSectionModel[] = []
  for (const bundle of bundles) {
    const all = (byBundle.get(bundle.key) ?? []).sort((a, b) => b.date - a.date || b.message.id - a.message.id)
    if (bundle.presentation === 'shelf') {
      const tiles = shelfTiles(all, now)
      if (tiles.length > 0) sections.push({ bundle, rows: all, allRows: all, sentence: '', tiles })
      continue
    }
    const cutoff = now.getTime() / 1000 - BUNDLE_WINDOW_DAYS * 86_400
    const rows = all.filter((r) => r.date >= cutoff)
    if (rows.length === 0) continue
    const fresh = rows.filter((r) => r.isNew).length
    const recent = `${rows.length} in den letzten ${BUNDLE_WINDOW_DAYS} Tagen, zuletzt von ${rows[0].sender}`
    sections.push({
      bundle,
      rows,
      allRows: all,
      tiles: [],
      sentence: fresh > 0 ? `${fresh} neu ${lowerFirst(input.sinceLabel)} · ${recent}` : recent,
    })
  }
  return sections
}

function shelfTiles(newestFirst: BundleRow[], now: Date): ShelfTile[] {
  const bySender = new Map<string, BundleRow[]>()
  for (const row of newestFirst) {
    const key = (row.message.header.from_addr ?? row.sender).toLowerCase()
    const list = bySender.get(key) ?? []
    list.push(row)
    bySender.set(key, list)
  }
  return [...bySender.entries()].map(([key, rows]) => ({
    key,
    name: rows[0].sender,
    brandEmail: rows[0].brandEmail,
    label: rows[0].subject,
    since: readySince(rows[0].date, now),
    newest: rows[0],
    older: rows.slice(1),
  }))
}

/** Time column of a row. */
export function rowTime(row: BundleRow): string {
  return listDate(row.date)
}
