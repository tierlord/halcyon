// Builds the model of the "Heute" start view from data the app already loads.
// Pure functions only: the view (Today.svelte) renders, App.svelte wires actions.

import type { CaseItem, CaseEvent, CaseStatus } from './cases'
import { holdsBack } from './phishing'
import type {
  MessageSummary,
  MessageTurnInfo,
  SourceRecord,
} from './types'

const DAY_MS = 86_400_000
const SINCE_KEY = 'halcyon.todayLastVisit'

/** After this many days without a delivery mail we ask whether the parcel arrived. */
export const ARRIVED_HINT_DAYS = 5
/** After this many days without a shipping mail we point it out quietly. */
export const NO_SHIPPING_HINT_DAYS = 7

// ----- "since when" ----------------------------------------------------------

/**
 * The moment the user last looked at Heute (unix seconds). A visit within the
 * last hour does not count: "since five minutes ago" would almost always be
 * empty, so we show the last day instead.
 */
export function loadSince(fallbackSecs = 24 * 3600): number {
  const nowSec = Math.floor(Date.now() / 1000)
  let stored = 0
  try {
    stored = Number(localStorage.getItem(SINCE_KEY)) || 0
  } catch {
    // Storage may be blocked; fall back to the default window.
  }
  if (!stored || nowSec - stored < 3600) return nowSec - fallbackSecs
  return stored
}

export function saveSince(): void {
  try {
    localStorage.setItem(SINCE_KEY, String(Math.floor(Date.now() / 1000)))
  } catch {
    // Ignore storage errors.
  }
}

/** "Seit gestern Abend", "Seit Montag", "In den letzten 24 Stunden". */
export function sinceLabel(sinceSec: number, now = new Date()): string {
  const since = new Date(sinceSec * 1000)
  const hours = (now.getTime() - since.getTime()) / 3_600_000
  if (hours <= 25 && hours >= 23.5) return 'In den letzten 24 Stunden'
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
  const dayDiff = Math.round((startOfToday - new Date(since.getFullYear(), since.getMonth(), since.getDate()).getTime()) / DAY_MS)
  const h = since.getHours()
  const part = h < 11 ? 'Morgen' : h < 14 ? 'Mittag' : h < 18 ? 'Nachmittag' : 'Abend'
  if (dayDiff === 0) return hours < 3 ? 'In den letzten Stunden' : `Seit heute ${part === 'Abend' ? 'Abend' : part}`
  if (dayDiff === 1) return `Seit gestern ${part}`
  if (dayDiff < 7) {
    const wd = new Intl.DateTimeFormat('de-DE', { weekday: 'long' }).format(since)
    return `Seit ${wd}`
  }
  return 'In den letzten Tagen'
}

export function greeting(now = new Date()): string {
  const h = now.getHours()
  if (h >= 5 && h < 11) return 'Guten Morgen'
  if (h >= 11 && h < 18) return 'Guten Tag'
  return 'Guten Abend'
}

export function dateLine(now = new Date()): string {
  return new Intl.DateTimeFormat('de-DE', { weekday: 'long', day: 'numeric', month: 'long' }).format(now)
}

const NUMBER_WORDS = ['keine', 'eine', 'zwei', 'drei', 'vier', 'fünf', 'sechs', 'sieben', 'acht', 'neun', 'zehn']
function numberWord(n: number): string {
  return n >= 0 && n < NUMBER_WORDS.length ? NUMBER_WORDS[n] : String(n)
}

// ----- helpers -----------------------------------------------------------------

function dayToDate(day: string): Date {
  return new Date(`${day}T12:00:00`)
}

/** Whole days between a YYYY-MM-DD day (local) and today. */
export function daysAgo(day: string, now = new Date()): number {
  const todayNoon = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 12).getTime()
  return Math.round((todayNoon - dayToDate(day).getTime()) / DAY_MS)
}

export function relativeDays(n: number): string {
  if (n <= 0) return 'heute'
  if (n === 1) return 'gestern'
  return `vor ${n} Tagen`
}

/** Text without invisible characters: what a mail shows is not what it contains. */
function visible(s: string): string {
  return s.replace(/[\u00ad\u034f\u180e\u200b-\u200f\u202a-\u202e\u2060-\u2064\u2066-\u2069\ufeff]/g, '')
}

function senderName(m: MessageSummary): string {
  return visible(m.header.from_name?.trim() || m.header.from_addr?.split('@')[0] || 'Jemand')
}

function normalizedSubject(m: MessageSummary): string {
  return (m.header.subject ?? '').replace(/^\s*((re|aw|fwd?|wg)\s*:\s*)+/i, '').trim().toLowerCase()
}

function recipientName(m: MessageSummary): string {
  const raw = m.header.to[0] ?? ''
  const match = raw.match(/^\s*"?([^"<]+?)"?\s*<([^>]+)>/)
  if (match) return match[1].trim()
  const [local, host] = raw.split('@')
  if (host && /^(no-?reply|info|service|support|kontakt|hello|hallo)/i.test(local)) {
    const brand = host.split('.')[0]
    return brand.charAt(0).toUpperCase() + brand.slice(1)
  }
  return local || 'der Gegenseite'
}

// ----- model -------------------------------------------------------------------

export interface TodayInput {
  cases: CaseItem[]
  sources: SourceRecord[]
  turns: Map<number, MessageTurnInfo>
  /** Every mail the app knows, incl. mail of cases and sent replies. */
  pool: MessageSummary[]
  /** Unix seconds. */
  since: number
  firstName: string | null
  now?: Date
}

/** What "×" does for an item; App.svelte maps it to the persistent action. */
export type Dismissal =
  | { kind: 'case'; caseId: number; previous: CaseStatus }
  | { kind: 'anomaly'; sourceId: number }
  | { kind: 'turn'; messageId: number }

export interface ForYouItem {
  key: string
  kind: 'case' | 'anomaly' | 'turn'
  headline: string
  sub: string | null
  /** Second quiet line, e.g. the message preview. */
  detail: string | null
  why: string
  /** For the brand logo; falls back to initials of `name`. */
  brandEmail: string | null
  name: string
  openId: number | null
  /** Mail to reply to; null hides the "Antworten" action. */
  replyTo: MessageSummary | null
  caseId: number | null
  sourceId: number | null
  dismissal: Dismissal
}

export interface OrderEventRow {
  id: number
  /** "12. Nov." */
  date: string
  label: string
  /** Null when the mail is gone: no link is offered then. */
  messageId: number | null
}

export interface OrderItem {
  caseItem: CaseItem
  sentence: string
  /** Index of the furthest known step: 0 Bestellt … 3 Angekommen. */
  step: number
  /** Short status label for the chip (one of ORDER_STEPS). */
  chip: string
  /** Latest mail of the case ("Mail öffnen"). */
  openId: number | null
  hint: string | null
  askArrived: boolean
  /** Finished within the last days: shown muted at the end of the list. */
  arrived: boolean
  /** Chronological, with dates. */
  timeline: OrderEventRow[]
}

/** Orders of one company, shown as a single (stacked) tile in the strip. */
export interface OrderGroup {
  /** Stable across status changes: the company's domain, else its name. */
  key: string
  name: string
  /** Address for the brand logo; null when the company has no known domain. */
  brandEmail: string | null
  /** Active orders first (newest first), recently arrived ones after. */
  orders: OrderItem[]
  /** Every order of the group is already delivered: shown muted at the end. */
  arrived: boolean
  /** An order has a hint (late, no shipping mail): shown as a quiet dot. */
  attention: boolean
}

export interface QuietItem {
  key: string
  icon: 'send' | 'warning'
  text: string
  sub: string | null
  openId: number | null
  /** "Nachhaken" action. */
  followUp: MessageSummary | null
  dismissSourceId: number | null
  dismissal: Dismissal
}

export interface AsUsualGroup {
  label: string
  count: number
}

export interface AsUsualRow {
  message: MessageSummary
  sender: string
  subject: string
}

/** Mail the phishing check holds back: not counted, not a task, no card. */
export interface HeldBack {
  /** "1 verdächtige Mail zurückgehalten" */
  summary: string
  rows: AsUsualRow[]
}

export interface TodayModel {
  dateLine: string
  greeting: string
  summary: string
  forYou: ForYouItem[]
  orders: OrderItem[]
  orderGroups: OrderGroup[]
  quiet: QuietItem[]
  asUsual: { summary: string; rows: AsUsualRow[] } | null
  /** One quiet line at the very end; null when nothing is held back. */
  held: HeldBack | null
}

export const ORDER_STEPS = ['Bestellt', 'Versendet', 'In Zustellung', 'Angekommen'] as const

/** Which step of the delivery an event proves; -1 if it says nothing about it. */
function eventStep(e: CaseEvent): number {
  const label = e.label.toLowerCase()
  if (e.kind === 'order.delivered' || /zugestellt|angekommen|geliefert/.test(label)) return 3
  if (/in zustellung|zustellung heute|wird zugestellt|ausgeliefert|zustellbasis/.test(label)) return 2
  if (e.kind === 'order.shipped' || /vers(a|e)nd|unterwegs/.test(label)) return 1
  if (e.kind === 'order.placed' || /bestellt|bestellung/.test(label) || e.phase === 'start') return 0
  return -1
}

function latestMessageId(c: CaseItem, incomingOnly: boolean): number | null {
  for (let i = c.events.length - 1; i >= 0; i--) {
    const e = c.events[i]
    if (e.messageId != null && (!incomingOnly || e.direction !== 'out')) return e.messageId
  }
  return c.messageIds[c.messageIds.length - 1] ?? null
}

const DAY_FORMAT = new Intl.DateTimeFormat('de-DE', { day: 'numeric', month: 'short' })

function orderTimeline(c: CaseItem): OrderEventRow[] {
  return [...c.events]
    .sort((a, b) => a.day.localeCompare(b.day) || a.id - b.id)
    .map((e) => ({
      id: e.id,
      date: DAY_FORMAT.format(dayToDate(e.day)),
      label: e.label,
      messageId: e.messageId ?? null,
    }))
}

/** Orders marked done stay visible this many days so a delivery does not just vanish. */
export const ARRIVED_VISIBLE_DAYS = 3

function buildArrivedOrder(c: CaseItem): OrderItem {
  return {
    caseItem: c,
    sentence: `Dein Paket von ${c.counterparty} ist angekommen.`,
    step: 3,
    chip: ORDER_STEPS[3],
    openId: latestMessageId(c, false),
    hint: null,
    askArrived: false,
    arrived: true,
    timeline: orderTimeline(c),
  }
}

function isRecentlyArrived(c: CaseItem, now: Date): boolean {
  if (c.kind !== 'order' || c.status !== 'done') return false
  // The day of the last mail, not `updatedAt`: that also moves when old mail
  // is processed later (backfill), which made year-old orders look fresh.
  const last = c.events.at(-1)?.day
  if (last != null) return daysAgo(last, now) <= ARRIVED_VISIBLE_DAYS
  return c.updatedAt != null && now.getTime() / 1000 - c.updatedAt <= ARRIVED_VISIBLE_DAYS * 86_400
}

function buildOrder(c: CaseItem, now: Date): OrderItem {
  const steps = c.events.map((e) => ({ e, s: eventStep(e) }))
  const step = Math.max(0, ...steps.map((x) => x.s))
  const top = steps.filter((x) => x.s === step).at(-1)?.e
  const placed = steps.find((x) => x.s === 0)?.e.day ?? c.orderDate ?? c.events[0]?.day
  const topAge = top ? daysAgo(top.day, now) : 0
  const placedAge = placed ? daysAgo(placed, now) : 0
  const cp = c.counterparty

  let sentence: string
  if (step === 0) sentence = `Bestellt bei ${cp}. Der Versand steht noch aus.`
  else if (step === 1) sentence = `${cp} hat dein Paket versendet, ${relativeDays(topAge)}.`
  else sentence = `Dein Paket von ${cp} ist in Zustellung.`

  let hint: string | null = null
  let askArrived = false
  if ((step === 1 || step === 2) && topAge > ARRIVED_HINT_DAYS) {
    hint = 'Sollte inzwischen angekommen sein?'
    askArrived = true
  } else if (step === 0 && placedAge > NO_SHIPPING_HINT_DAYS) {
    hint = `Seit ${placedAge} Tagen bestellt, noch keine Versandmail.`
  }
  return {
    caseItem: c,
    sentence,
    step,
    chip: ORDER_STEPS[step],
    openId: latestMessageId(c, false),
    hint,
    askArrived,
    arrived: false,
    timeline: orderTimeline(c),
  }
}

/** Groups orders by company, keeping the order of first appearance (active before arrived). */
export function groupOrders(orders: OrderItem[]): OrderGroup[] {
  const groups = new Map<string, OrderGroup>()
  for (const o of orders) {
    const c = o.caseItem
    const key = (c.domain ?? c.counterparty).trim().toLowerCase()
    let g = groups.get(key)
    if (!g) {
      g = {
        key,
        name: c.counterparty,
        brandEmail: c.domain ? `info@${c.domain}` : null,
        orders: [],
        arrived: true,
        attention: false,
      }
      groups.set(key, g)
    }
    g.orders.push(o)
    if (!o.arrived) g.arrived = false
    if (o.hint && !o.arrived) g.attention = true
  }
  // active orders first inside a group (stable sort keeps the recency order)
  for (const g of groups.values()) g.orders.sort((a, b) => Number(a.arrived) - Number(b.arrived))
  return [...groups.values()]
}

function isActiveOrder(c: CaseItem) {
  return c.kind === 'order' && c.status !== 'done' && c.status !== 'rejected' && c.status !== 'aside'
}

const LOGIN_RE = /anmeldung|sign-?in|log-?in|code|passwort|password|verif|bestätigung/i
const PROMO_RE = /%|angebot|rabatt|sale|gutschein|nur heute|aktion|deal/i

function plural(n: number, one: string, many: string) {
  return n === 1 ? one : many
}

function joinWords(parts: string[]): string {
  if (parts.length <= 1) return parts.join('')
  return `${parts.slice(0, -1).join(', ')} und ${parts.at(-1)}`
}

function sourceFor(m: MessageSummary, sources: SourceRecord[]): SourceRecord | null {
  const addr = m.header.from_addr?.toLowerCase() ?? ''
  const host = addr.split('@')[1] ?? ''
  if (!host) return null
  return (
    sources.find((s) => s.source_key.toLowerCase() === addr) ??
    sources.find((s) => s.domain && (host === s.domain || host.endsWith(`.${s.domain}`))) ??
    null
  )
}

export function buildToday(input: TodayInput): TodayModel {
  const now = input.now ?? new Date()
  const { cases, sources, turns, pool, since } = input
  const byId = new Map(pool.map((m) => [m.id, m]))
  const msg = (id: number | null) => (id != null ? (byId.get(id) ?? null) : null)

  const shown = new Set<number>() // mail already represented by a card or line

  // --- Für dich ---
  const forYou: ForYouItem[] = []

  for (const c of cases.filter((c) => c.status === 'needs_you')) {
    const incoming = latestMessageId(c, true)
    const m = msg(incoming)
    // A lure can look like an invoice or a deadline and make its own case;
    // that is no task for the user.
    if (m && holdsBack(m)) continue
    let headline: string
    let sub: string | null = c.title
    if (c.amount && (c.kind === 'invoice' || c.kind === 'contract')) {
      const due = c.dueLabel
        ? /^(in |heute|morgen|übermorgen)/i.test(c.dueLabel)
          ? `, fällig ${c.dueLabel}`
          : ` bis ${c.dueLabel}`
        : ''
      headline = `${c.counterparty} möchte ${c.amount}${due}`
    } else if (c.kind === 'conversation') {
      headline = `${c.counterparty} wartet auf deine Antwort`
    } else {
      headline = `${c.counterparty} braucht dich bei „${c.title}“`
      sub = c.nextStep
    }
    c.messageIds.forEach((id) => shown.add(id))
    forYou.push({
      key: `case-${c.id}`,
      kind: 'case',
      headline,
      sub,
      detail: null,
      why: c.reason,
      brandEmail: c.domain ? `info@${c.domain}` : null,
      name: c.counterparty,
      openId: incoming,
      replyTo: c.kind === 'conversation' ? m : null,
      caseId: c.id,
      sourceId: null,
      dismissal: { kind: 'case', caseId: c.id, previous: c.status },
    })
  }

  for (const s of sources.filter((s) => s.anomaly?.needs_you)) {
    if (s.latest_message_id != null) shown.add(s.latest_message_id)
    forYou.push({
      key: `anomaly-${s.id}`,
      kind: 'anomaly',
      headline: s.anomaly!.reason,
      sub: s.display_name,
      detail: null,
      why: s.anomaly!.reason,
      brandEmail: s.domain ? `info@${s.domain}` : null,
      name: s.display_name,
      openId: s.latest_message_id,
      replyTo: null,
      caseId: null,
      sourceId: s.id,
      dismissal: { kind: 'anomaly', sourceId: s.id },
    })
  }

  const caseMsgIds = new Set(cases.flatMap((c) => c.messageIds))
  const seenThreads = new Set<string>()
  // Only the newest mail of a thread counts: dismissing it must not make an
  // older, equally "needs you" mail of the same thread pop up instead.
  const newestOfThread = new Map<string, MessageSummary>()
  for (const m of pool) {
    if (m.folder_role === 'sent' || m.folder_role === 'trash' || m.folder_role === 'junk') continue
    const key = `${m.header.from_addr}|${normalizedSubject(m)}`
    const cur = newestOfThread.get(key)
    if (!cur || (m.header.date ?? 0) > (cur.header.date ?? 0) || ((m.header.date ?? 0) === (cur.header.date ?? 0) && m.id > cur.id)) {
      newestOfThread.set(key, m)
    }
  }
  const waitingTurns: { m: MessageSummary; t: MessageTurnInfo }[] = []
  for (const t of turns.values()) {
    const m = byId.get(t.message_id)
    if (!m || m.folder_role === 'trash' || m.folder_role === 'junk') continue
    if (t.status === 'waiting') {
      if (m.folder_role === 'sent') waitingTurns.push({ m, t })
      continue
    }
    if (t.status !== 'needs_you' || m.folder_role === 'sent') continue
    if (holdsBack(m)) continue
    if (caseMsgIds.has(m.id)) continue // the case card already covers it
    const thread = `${m.header.from_addr}|${normalizedSubject(m)}`
    if (seenThreads.has(thread) || newestOfThread.get(thread)?.id !== m.id) continue
    seenThreads.add(thread)
    shown.add(m.id)
    forYou.push({
      key: `turn-${m.id}`,
      kind: 'turn',
      headline: `${senderName(m)} wartet auf deine Antwort`,
      sub: m.header.subject ?? '(kein Betreff)',
      detail: m.preview,
      why: t.reason,
      brandEmail: m.header.from_addr,
      name: senderName(m),
      openId: m.id,
      replyTo: m,
      caseId: null,
      sourceId: null,
      dismissal: { kind: 'turn', messageId: m.id },
    })
  }

  // --- Unterwegs ---
  const active = cases
    .filter(isActiveOrder)
    .filter((c) => c.status !== 'needs_you')
    .map((c) => {
      c.messageIds.forEach((id) => shown.add(id))
      return buildOrder(c, now)
    })
    .sort((a, b) => {
      const da = a.caseItem.events.at(-1)?.day ?? ''
      const db = b.caseItem.events.at(-1)?.day ?? ''
      return db.localeCompare(da)
    })
  const arrived = cases
    .filter((c) => isRecentlyArrived(c, now))
    .map((c) => buildArrivedOrder(c))
    .sort((a, b) => (b.caseItem.events.at(-1)?.day ?? '').localeCompare(a.caseItem.events.at(-1)?.day ?? ''))
  const orders = [...active, ...arrived]

  // --- Läuft von allein ---
  const quiet: QuietItem[] = []
  for (const c of cases.filter((c) => c.kind !== 'order' && c.status === 'waiting')) {
    c.messageIds.forEach((id) => shown.add(id))
    quiet.push({
      key: `case-${c.id}`,
      icon: 'send',
      text: c.title,
      sub: c.nextStep,
      openId: latestMessageId(c, false),
      followUp: null,
      dismissSourceId: null,
      dismissal: { kind: 'case', caseId: c.id, previous: c.status },
    })
  }
  for (const { m, t } of waitingTurns) {
    shown.add(m.id)
    const who = recipientName(m)
    quiet.push({
      key: `turn-${m.id}`,
      icon: 'send',
      text: t.follow_up_due
        ? `Seit ein paar Tagen wartest du auf eine Antwort von ${who}`
        : `Du wartest auf eine Antwort von ${who}`,
      sub: m.header.subject ?? null,
      openId: m.id,
      followUp: t.follow_up_due ? m : null,
      dismissSourceId: null,
      dismissal: { kind: 'turn', messageId: m.id },
    })
  }
  for (const s of sources.filter((s) => s.anomaly && !s.anomaly.needs_you)) {
    quiet.push({
      key: `anomaly-${s.id}`,
      icon: 'warning',
      text: `${s.display_name}: ${s.anomaly!.reason}`,
      sub: null,
      openId: s.latest_message_id,
      followUp: null,
      dismissSourceId: s.id,
      dismissal: { kind: 'anomaly', sourceId: s.id },
    })
  }

  // --- Wie immer ---
  const recent = pool
    .filter((m) => m.folder_role === 'inbox' && (m.header.date ?? 0) >= since)
    .sort((a, b) => (b.header.date ?? 0) - (a.header.date ?? 0))
  // Held-back mail is neither counted nor sorted into a group: it must not
  // look like part of the day, only be findable.
  const fresh = recent.filter((m) => !holdsBack(m))
  const heldMails = recent.filter((m) => holdsBack(m))
  const held: HeldBack | null =
    heldMails.length > 0
      ? {
          summary: `${heldMails.length} ${plural(heldMails.length, 'verdächtige Mail', 'verdächtige Mails')} zurückgehalten`,
          rows: heldMails.map((m) => ({
            message: m,
            sender: senderName(m),
            subject: visible(m.header.subject ?? '(kein Betreff)'),
          })),
        }
      : null
  const rest = fresh.filter((m) => !shown.has(m.id))
  const counts = new Map<string, number>()
  const bump = (k: string) => counts.set(k, (counts.get(k) ?? 0) + 1)
  for (const m of rest) {
    const src = sourceFor(m, sources)
    const subject = m.header.subject ?? ''
    const newsletter = src?.is_newsletter ?? (m.header.list_id != null || m.header.list_unsubscribe.length > 0)
    if (newsletter) bump(PROMO_RE.test(subject) ? 'promo' : 'newsletter')
    else if (LOGIN_RE.test(subject)) bump('login')
    else if (src && src.mode !== 'show') bump('notice')
    else bump('other')
  }
  const labelFor: Record<string, (n: number) => string> = {
    newsletter: (n) => `${n} Newsletter`,
    promo: (n) => `${n} ${plural(n, 'Werbemail', 'Werbemails')}`,
    login: (n) => `${n} ${plural(n, 'Login-Hinweis', 'Login-Hinweise')}`,
    notice: (n) => `${n} ${plural(n, 'automatischer Hinweis', 'automatische Hinweise')}`,
    other: (n) => `${n} ${plural(n, 'weitere Mail', 'weitere Mails')}`,
  }
  const groups = [...counts.entries()].sort((a, b) => b[1] - a[1])
  const asUsual =
    rest.length > 0
      ? {
          summary: `Wie immer: ${joinWords(groups.map(([k, n]) => labelFor[k](n)))}`,
          rows: rest.map((m) => ({
            message: m,
            sender: senderName(m),
            subject: visible(m.header.subject ?? '(kein Betreff)'),
          })),
        }
      : null

  // --- Greeting ---
  const n = fresh.length
  const when = sinceLabel(since, now)
  const first =
    n === 0
      ? `${when} ist nichts Neues gekommen.`
      : `${when} ${n === 1 ? 'ist eine Mail' : `sind ${n} Mails`} gekommen.`
  const nYou = forYou.length
  let second: string
  if (nYou === 0) second = n === 0 ? 'Nichts Dringendes.' : 'Nichts Dringendes. Der Rest ist wie immer.'
  else {
    const what = nYou === 1 ? 'Eine Sache braucht' : `${capitalize(numberWord(nYou))} Dinge brauchen`
    second = n === 0 ? `${what} dich.` : `${what} dich, der Rest ist wie immer.`
  }
  const name = input.firstName
  return {
    dateLine: dateLine(now),
    greeting: name ? `${greeting(now)}, ${name}` : greeting(now),
    summary: `${first} ${second}`,
    forYou,
    orders,
    orderGroups: groupOrders(orders),
    quiet,
    asUsual,
    held,
  }
}

function capitalize(s: string) {
  return s.charAt(0).toUpperCase() + s.slice(1)
}
