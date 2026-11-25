// Browser mock of the bundle and assignment commands (see api.ts). It keeps
// everything in memory and follows the rules of halcyon-core loosely:
// automatic placement by simple signals, previews grouped by a subject head,
// protected mail (tracking or invoice number, one-time code, answered), and
// assignments for one mail or for a kind of mail with exceptions.

import { holdsBack } from './phishing'
import * as mock from './mock'
import {
  OTHER_KEY,
  type Assignment,
  type AssignmentPreview,
  type Bundle,
  type BundlePatch,
  type Placement,
  type PreviewGroup,
  type Target,
} from './bundles'
import type { MessageSummary } from './types'

const delay = <T>(value: T, ms = 70) => new Promise<T>((r) => setTimeout(() => r(value), ms))

let bundleList: Bundle[] = [
  { key: 'overviews', name: 'Übersichten', builtin: true, position: 0, enabled: true, presentation: 'shelf', mark_read: false },
  { key: 'account', name: 'Konto & Hinweise', builtin: true, position: 1, enabled: true, presentation: 'collapsed', mark_read: false },
  { key: 'newsletter', name: 'Newsletter & Aktuelles', builtin: true, position: 2, enabled: true, presentation: 'collapsed', mark_read: false },
  { key: 'promo', name: 'Werbung', builtin: true, position: 3, enabled: true, presentation: 'collapsed', mark_read: true },
  { key: 'social', name: 'Soziales', builtin: true, position: 4, enabled: false, presentation: 'collapsed', mark_read: false },
  { key: OTHER_KEY, name: 'Weitere Mails', builtin: true, position: 1000, enabled: true, presentation: 'collapsed', mark_read: false },
]

interface Stored {
  id: number
  scope: 'mail' | 'kind'
  target: Target
  createdAt: number
  /** scope mail */
  messageId?: number
  /** scope kind */
  sender?: string
  kindKey?: string
  exceptions: Set<number>
}

let stored: Stored[] = []
let nextId = 1

// ----- mail facts ----------------------------------------------------------------

function considered(m: MessageSummary): boolean {
  return m.folder_role !== 'sent' && m.folder_role !== 'trash' && m.folder_role !== 'junk'
}

function text(m: MessageSummary): string {
  return `${m.header.subject ?? ''} ${m.preview ?? ''}`
}

/** The part of a subject that names the kind of mail: "Angebot: Lampe" -> "Angebot". */
function subjectHead(subject: string): string {
  const s = subject.replace(/^\s*((re|aw|fwd?|wg)\s*:\s*)+/i, '').trim()
  const colon = s.indexOf(':')
  const head = colon > 0 ? s.slice(0, colon) : (s.split(/\s+/)[0] ?? s)
  return head.replace(/[\d.,]+$/, '').trim() || s
}

const HEAD_LABELS: Record<string, string> = {
  angebot: 'Angebote',
  versandbestätigung: 'Versandbestätigungen',
  nachricht: 'Nachrichten',
  kontoauszug: 'Kontoauszüge',
}

function kindOf(m: MessageSummary): { key: string; label: string } {
  const head = subjectHead(m.header.subject ?? '')
  const key = head.toLowerCase().replace(/[^\p{L}]+/gu, '-')
  return { key, label: HEAD_LABELS[key] ?? head }
}

function sentRepliesTo(m: MessageSummary): boolean {
  const id = m.header.message_id
  if (!id) return false
  return mock.allMockMessages().some((x) => x.folder_role === 'sent' && x.header.in_reply_to === id)
}

function protection(m: MessageSummary): string[] {
  const t = text(m)
  const reasons: string[] = []
  if (/(einmal|bestätigungs|sicherheits)?code:?\s*\d{4,8}/i.test(t)) reasons.push('enthält einen Einmalcode')
  if (/sendungsnummer|\b\d{12,}\b/i.test(t)) reasons.push('enthält eine Sendungsnummer')
  if (/rechnung(snummer)?\s*(nr\.?\s*)?[\d-]{4,}/i.test(t)) reasons.push('enthält eine Rechnung')
  if (sentRepliesTo(m)) reasons.push('du hast darauf geantwortet')
  return reasons
}

function autoBundle(m: MessageSummary): { key: string; reason: string } | null {
  const s = m.header.subject ?? ''
  if (m.header.list_id != null || m.header.list_unsubscribe.length > 0) {
    return { key: 'newsletter', reason: 'Enthält einen Abmelde-Link' }
  }
  if (/%|angebot|rabatt|\bsale\b|gutschein|nur heute|aktion|deals?\b|gewinn/i.test(s)) {
    return { key: 'promo', reason: 'Klingt nach Werbung' }
  }
  if (/kontoauszug|wochenübersicht|depotauszug|verbrauchsübersicht|jahresübersicht/i.test(s)) {
    return { key: 'overviews', reason: 'Regelmäßiger Auszug' }
  }
  if (/anmeldung|passwort|kennwort|bedingungen|neue dokumente/i.test(s)) {
    return { key: 'account', reason: 'Hinweis zu deinem Konto' }
  }
  return null
}

function bundleName(key: string | null): string | null {
  return key ? (bundleList.find((b) => b.key === key)?.name ?? null) : null
}

function enabledKey(key: string): string {
  return bundleList.some((b) => b.key === key && b.enabled) ? key : OTHER_KEY
}

function assignedFor(m: MessageSummary): { st: Stored; origin: 'mail' | 'kind' } | null {
  const byMail = stored.find((s) => s.scope === 'mail' && s.messageId === m.id)
  if (byMail) return { st: byMail, origin: 'mail' }
  const sender = m.header.from_addr ?? ''
  const kind = kindOf(m).key
  const byKind = stored.find(
    (s) => s.scope === 'kind' && s.sender === sender && s.kindKey === kind && !s.exceptions.has(m.id),
  )
  return byKind ? { st: byKind, origin: 'kind' } : null
}

function placementOf(m: MessageSummary): Placement {
  const kind = kindOf(m)
  const reasons = protection(m)
  const base = {
    message_id: m.id,
    assigned_at: null as number | null,
    kind_key: kind.key,
    kind_label: kind.label,
    protected: reasons.length > 0,
    protection: reasons,
  }
  if (holdsBack(m)) {
    return { ...base, target: null, origin: 'suspicious', held_back: null, bundle: null, bundle_name: null, reason: 'Von der Phishing-Prüfung zurückgehalten' }
  }
  const hit = assignedFor(m)
  if (hit) {
    const { st, origin } = hit
    // A kind assignment never reaches protected mail; the one for this mail does.
    if (origin === 'kind' && reasons.length > 0 && st.target.kind === 'bundle') {
      return { ...base, target: null, origin: null, held_back: st.target, bundle: null, bundle_name: null, reason: `Bleibt sichtbar: ${reasons[0]}` }
    }
    const key = st.target.kind === 'bundle' ? enabledKey(st.target.bundle) : null
    const name = key ? bundleName(key) : 'Für dich'
    return {
      ...base,
      target: st.target.kind === 'bundle' && key !== st.target.bundle ? null : st.target,
      origin,
      assigned_at: st.createdAt,
      held_back: null,
      bundle: key,
      bundle_name: key ? name : null,
      reason: `Von dir in ‚${name}' eingeordnet`,
    }
  }
  if (reasons.length === 0) {
    const auto = autoBundle(m)
    if (auto) {
      const key = enabledKey(auto.key)
      return { ...base, target: null, origin: 'auto', held_back: null, bundle: key, bundle_name: bundleName(key), reason: auto.reason }
    }
  }
  return { ...base, target: null, origin: null, held_back: null, bundle: null, bundle_name: null, reason: null }
}

// ----- commands ------------------------------------------------------------------

export function placements(messageIds: number[]): Promise<Placement[]> {
  const byId = new Map(mock.allMockMessages().map((m) => [m.id, m]))
  const out: Placement[] = []
  for (const id of messageIds) {
    const m = byId.get(id)
    if (m) out.push(placementOf(m))
  }
  return delay(out, 40)
}

export function assignmentPreview(messageId: number, target: Target): Promise<AssignmentPreview> {
  const all = mock.allMockMessages()
  const own = all.find((m) => m.id === messageId)
  if (!own) return Promise.reject(new Error('Die Mail gibt es nicht mehr.'))
  const sender = own.header.from_addr ?? ''
  const ownKind = kindOf(own).key
  const mails = all.filter((m) => considered(m) && (m.header.from_addr ?? '') === sender)
  const byKind = new Map<string, MessageSummary[]>()
  for (const m of mails) {
    const k = kindOf(m).key
    byKind.set(k, [...(byKind.get(k) ?? []), m])
  }
  const groups: PreviewGroup[] = [...byKind.entries()].map(([key, list]) => {
    const newest = [...list].sort((a, b) => (b.header.date ?? 0) - (a.header.date ?? 0))
    const dates = newest.map((m) => m.header.date ?? 0)
    const prot = newest.filter((m) => protection(m).length > 0)
    const protIds = new Set(prot.map((m) => m.id))
    return {
      kind_key: key,
      label: kindOf(newest[0]).label,
      features: prot.length > 0 ? ['tracking'] : [],
      count: newest.length,
      protected_count: prot.length,
      examples: newest.slice(0, 3).map((m) => m.header.subject ?? ''),
      first_date: Math.min(...dates),
      last_date: Math.max(...dates),
      preselected: key === ownKind,
      message_ids: newest.filter((m) => !protIds.has(m.id)).map((m) => m.id),
      protected: prot.map((m) => ({
        message_id: m.id,
        subject: m.header.subject ?? '',
        date: m.header.date ?? 0,
        reasons: protection(m),
      })),
      assigned: stored.find((s) => s.scope === 'kind' && s.sender === sender && s.kindKey === key)?.target ?? null,
    }
  })
  groups.sort((a, b) => b.count - a.count)
  const selected = groups.filter((g) => g.preselected)
  return delay({
    message_id: messageId,
    sender,
    sender_name: own.header.from_name,
    target,
    kind_key: ownKind,
    groups,
    total_mails: mails.length,
    group_count: groups.length,
    selected_mails: selected.reduce((n, g) => n + g.message_ids.length, 0),
    selected_protected: selected.reduce((n, g) => n + g.protected_count, 0),
    other_mails: groups.filter((g) => !g.preselected).reduce((n, g) => n + g.count, 0),
    protected_total: groups.reduce((n, g) => n + g.protected_count, 0),
  })
}

export function applyAssignment(
  messageId: number,
  target: Target,
  kindKeys: string[],
  excludedMessageIds: number[],
): Promise<number[]> {
  const own = mock.allMockMessages().find((m) => m.id === messageId)
  if (!own) return Promise.reject(new Error('Die Mail gibt es nicht mehr.'))
  if (target.kind === 'bundle' && !bundleList.some((b) => b.key === target.bundle)) {
    return Promise.reject(new Error('Dieses Bündel gibt es nicht.'))
  }
  const ids: number[] = []
  const now = Math.floor(Date.now() / 1000)
  if (kindKeys.length === 0) {
    stored = stored.filter((s) => !(s.scope === 'mail' && s.messageId === messageId))
    const st: Stored = { id: nextId++, scope: 'mail', target, createdAt: now, messageId, exceptions: new Set() }
    stored.push(st)
    ids.push(st.id)
  } else {
    const sender = own.header.from_addr ?? ''
    for (const key of kindKeys) {
      stored = stored.filter((s) => !(s.scope === 'kind' && s.sender === sender && s.kindKey === key))
      const inKind = new Set(
        mock
          .allMockMessages()
          .filter((m) => (m.header.from_addr ?? '') === sender && kindOf(m).key === key)
          .map((m) => m.id),
      )
      const st: Stored = {
        id: nextId++,
        scope: 'kind',
        target,
        createdAt: now,
        sender,
        kindKey: key,
        exceptions: new Set(excludedMessageIds.filter((id) => inKind.has(id))),
      }
      stored.push(st)
      ids.push(st.id)
    }
  }
  return delay(ids)
}

export function removeAssignment(id: number): Promise<void> {
  stored = stored.filter((s) => s.id !== id)
  return delay(undefined, 30)
}

export function assignments(): Promise<Assignment[]> {
  const all = mock.allMockMessages()
  return delay(
    [...stored].reverse().map((s) => {
      const m = s.messageId != null ? all.find((x) => x.id === s.messageId) : undefined
      return {
        id: s.id,
        scope: s.scope,
        target: s.target,
        created_at: s.createdAt,
        rfc_message_id: m?.header.message_id ?? null,
        sender: s.sender ?? null,
        kind_key: s.kindKey ?? null,
        kind_label: s.kindKey ?? null,
        subject: m?.header.subject ?? null,
        exceptions: s.exceptions.size,
      }
    }),
  )
}

// ----- bundles -------------------------------------------------------------------

function sorted(): Bundle[] {
  return [...bundleList]
    .sort((a, b) => Number(a.key === OTHER_KEY) - Number(b.key === OTHER_KEY) || a.position - b.position)
    .map((b) => ({ ...b }))
}

export function bundles(): Promise<Bundle[]> {
  return delay(sorted(), 30)
}

function slug(name: string): string {
  return (
    name
      .toLowerCase()
      .replace(/ä/g, 'ae')
      .replace(/ö/g, 'oe')
      .replace(/ü/g, 'ue')
      .replace(/ß/g, 'ss')
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '') || 'bundle'
  )
}

export function createBundle(name: string): Promise<Bundle> {
  const clean = name.trim()
  if (!clean) return Promise.reject(new Error('Das Bündel braucht einen Namen.'))
  let key = slug(clean) === 'for-you' ? 'bundle' : slug(clean)
  const base = key
  for (let n = 2; bundleList.some((b) => b.key === key); n++) key = `${base}-${n}`
  const above = bundleList.filter((b) => b.key !== OTHER_KEY)
  const bundle: Bundle = {
    key,
    name: clean.slice(0, 60),
    builtin: false,
    position: Math.max(-1, ...above.map((b) => b.position)) + 1,
    enabled: true,
    presentation: 'collapsed',
    mark_read: false,
  }
  bundleList = [...bundleList, bundle]
  return delay({ ...bundle })
}

export function updateBundle(key: string, patch: BundlePatch): Promise<Bundle> {
  const b = bundleList.find((x) => x.key === key)
  if (!b) return Promise.reject(new Error('Dieses Bündel gibt es nicht.'))
  if (patch.name !== undefined) {
    if (b.builtin) return Promise.reject(new Error('Eingebaute Bündel lassen sich nicht umbenennen.'))
    if (!patch.name.trim()) return Promise.reject(new Error('Das Bündel braucht einen Namen.'))
    b.name = patch.name.trim().slice(0, 60)
  }
  if (patch.enabled !== undefined) {
    if (key === OTHER_KEY) return Promise.reject(new Error('„Weitere Mails“ bleibt immer sichtbar.'))
    b.enabled = patch.enabled
  }
  if (patch.markRead !== undefined) {
    if (key === OTHER_KEY) return Promise.reject(new Error('Hier gibt es nichts als gelesen zu markieren.'))
    b.mark_read = patch.markRead
  }
  if (patch.presentation !== undefined) b.presentation = patch.presentation
  if (patch.position !== undefined && key !== OTHER_KEY) {
    // Same as the core: an index among the bundles above `other`, clamped.
    const keys = bundleList
      .filter((x) => x.key !== OTHER_KEY && x.key !== key)
      .sort((x, y) => x.position - y.position)
      .map((x) => x.key)
    keys.splice(Math.min(Math.max(patch.position, 0), keys.length), 0, key)
    keys.forEach((k, i) => {
      const x = bundleList.find((y) => y.key === k)
      if (x) x.position = i
    })
  }
  return delay({ ...b })
}

export function deleteBundle(key: string): Promise<void> {
  const b = bundleList.find((x) => x.key === key)
  if (!b || b.builtin) return Promise.reject(new Error('Eingebaute Bündel lassen sich nicht löschen.'))
  bundleList = bundleList.filter((x) => x.key !== key)
  stored = stored.filter((s) => !(s.target.kind === 'bundle' && s.target.bundle === key))
  return delay(undefined, 40)
}
