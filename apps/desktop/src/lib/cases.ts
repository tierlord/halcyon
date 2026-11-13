import type { MessageSummary } from './types'

export type CaseKind =
  | 'order'
  | 'contract'
  | 'invoice'
  | 'conversation'
  | 'travel'
  | 'security'
  | 'support'
  | 'other'

export type CaseStatus = 'needs_you' | 'waiting' | 'aside' | 'done' | 'rejected'

export interface CaseEvent {
  id: number
  /** ISO date YYYY-MM-DD for timeline alignment. */
  day: string
  label: string
  /** Role on the horizontal calendar: start, update, due, or done. */
  phase: 'start' | 'update' | 'due' | 'done'
  messageId?: number
  direction?: 'in' | 'out'
}

export interface CaseKey {
  label: string
  value: string
}

export interface CaseAttachment {
  id: string
  name: string
  sizeLabel?: string
  messageId?: number
}

export interface CaseItem {
  id: number
  title: string
  counterparty: string
  domain: string | null
  kind: CaseKind
  status: CaseStatus
  nextStep: string
  dueLabel?: string
  amount?: string
  reason: string
  keys: CaseKey[]
  events: CaseEvent[]
  attachments?: CaseAttachment[]
  messageIds: number[]
}

export interface AsideCategory {
  id: string
  label: string
  icon: 'tag' | 'warning' | 'mail'
  count: number
  summary: string
  messageIds: number[]
}

/** Formats a YYYY-MM-DD offset relative to today so mock data always stays current. */
export function dayOffset(days: number): string {
  const d = new Date()
  d.setHours(12, 0, 0, 0)
  d.setDate(d.getDate() + days)
  return d.toISOString().slice(0, 10)
}

export function caseKindLabel(kind: CaseKind): string {
  switch (kind) {
    case 'order':
      return 'Bestellung'
    case 'contract':
      return 'Vertrag'
    case 'invoice':
      return 'Rechnung'
    case 'conversation':
      return 'Gespräch'
    case 'travel':
      return 'Reise'
    case 'security':
      return 'Sicherheit'
    case 'support':
      return 'Support'
    case 'other':
      return 'Sonstiges'
  }
}

export function createMockCases(messages: MessageSummary[]): {
  cases: CaseItem[]
  categories: AsideCategory[]
} {
  const findId = (needle: RegExp) => messages.find((m) => needle.test(m.header.subject ?? ''))?.id
  const findIds = (needle: RegExp) =>
    messages.filter((m) => needle.test(m.header.subject ?? '') || needle.test(m.header.from_addr ?? '')).map((m) => m.id)

  const kleinanzeigenIds = findIds(/Rennrad 28 Zoll/)
  const stadtwerkeStrom = findId(/Abschlagsrechnung Strom/)
  const stadtwerkeJahr = findId(/Jahresabrechnung 2025/)
  const shopOrder = findId(/20 % auf alles/)
  const kranzIds = findIds(/Ersatzteile|Angebot vom Dienstag/)
  const reiseId = findId(/Hamburg/)
  const fotobuchId = findId(/Fotobuch/)
  const versicherungId = findId(/Vertrag Nr\. 2026-1184/)
  const bankId = findId(/Neue Dokumente/)

  const cases: CaseItem[] = [
    {
      id: 1,
      title: 'Rennrad 28 Zoll',
      counterparty: 'Kleinanzeigen · Interessent',
      domain: 'kleinanzeigen.example',
      kind: 'conversation',
      status: 'needs_you',
      nextStep: 'Rückfrage zur Abholung am Samstag beantworten',
      dueLabel: 'Heute',
      reason: 'Verknüpft über Anzeigentitel „Rennrad 28 Zoll“ und Thread-Header',
      keys: [
        { label: 'Anzeige', value: 'Rennrad 28 Zoll' },
        { label: 'Gegenüber', value: 'Kleinanzeigen' },
      ],
      events: [
        { id: 1, day: dayOffset(-2), label: 'Anzeige online', phase: 'start', messageId: kleinanzeigenIds[2], direction: 'in' },
        { id: 2, day: dayOffset(-1), label: 'Erste Anfrage', phase: 'update', messageId: kleinanzeigenIds[1], direction: 'in' },
        { id: 3, day: dayOffset(-1), label: 'Deine Antwort', phase: 'update', messageId: kleinanzeigenIds[3], direction: 'out' },
        { id: 4, day: dayOffset(0), label: 'Neue Rückfrage', phase: 'due', messageId: kleinanzeigenIds[0], direction: 'in' },
      ],
      messageIds: kleinanzeigenIds.filter((id): id is number => id != null),
    },
    {
      id: 2,
      title: 'Jahresabrechnung Strom',
      counterparty: 'Stadtwerke',
      domain: 'stadtwerke.example',
      kind: 'invoice',
      status: 'needs_you',
      nextStep: '148,50 € fällig bis übermorgen',
      dueLabel: 'in 2 Tagen',
      amount: '148,50 €',
      reason: 'Erkannt als Rechnung mit offenem Betrag und Fälligkeitsdatum',
      keys: [
        { label: 'Kundennr.', value: 'SW-884120' },
        { label: 'Betrag', value: '148,50 €' },
      ],
      events: [
        { id: 5, day: dayOffset(-3), label: 'Rechnung gestellt', phase: 'start', messageId: stadtwerkeJahr ?? stadtwerkeStrom, direction: 'in' },
        { id: 6, day: dayOffset(2), label: 'Fällig', phase: 'due' },
      ],
      attachments: [
        { id: 'att-sw-1', name: 'Jahresabrechnung_2025.pdf', sizeLabel: '420 KB', messageId: stadtwerkeJahr ?? stadtwerkeStrom },
      ],
      messageIds: [stadtwerkeJahr, stadtwerkeStrom].filter((id): id is number => id != null),
    },
    {
      id: 3,
      title: 'Angebot Ersatzteile',
      counterparty: 'Sabine Kranz · Zulieferer',
      domain: null,
      kind: 'support',
      status: 'waiting',
      nextStep: 'Wartet auf Auftragsbestätigung nach deiner Antwort',
      dueLabel: 'bis Fr',
      reason: 'Verknüpft über Thread-Header (In-Reply-To / References)',
      keys: [{ label: 'Kontakt', value: 'kranz@zulieferer.example' }],
      events: [
        { id: 7, day: dayOffset(-1), label: 'Angebot erhalten', phase: 'start', messageId: kranzIds[0], direction: 'in' },
        { id: 8, day: dayOffset(0), label: 'Freigabe gesendet', phase: 'update', messageId: kranzIds[1], direction: 'out' },
        { id: 9, day: dayOffset(3), label: 'Rückmeldung erwartet', phase: 'due' },
      ],
      attachments: [
        { id: 'att-kranz-1', name: 'Angebot_Ersatzteile.pdf', sizeLabel: '185 KB', messageId: kranzIds[0] },
      ],
      messageIds: kranzIds,
    },
    {
      id: 4,
      title: 'Hamburg 12.–14. Dez',
      counterparty: 'Reiseportal',
      domain: 'reise.example',
      kind: 'travel',
      status: 'waiting',
      nextStep: 'Buchung bestätigt · Abreise im Dezember',
      dueLabel: '12. Dez',
      reason: 'Reisebestätigung erkannt (Datum + Buchungsreferenz)',
      keys: [{ label: 'Reisezeitraum', value: '12.–14.12.2026' }],
      events: [
        { id: 10, day: dayOffset(-6), label: 'Gebucht', phase: 'start', messageId: reiseId, direction: 'in' },
        { id: 11, day: dayOffset(4), label: 'Ticket-Abruf', phase: 'due' },
      ],
      attachments: [
        { id: 'att-reise-1', name: 'Buchungsbestaetigung_HH.pdf', sizeLabel: '310 KB', messageId: reiseId },
      ],
      messageIds: reiseId != null ? [reiseId] : [],
    },
    {
      id: 5,
      title: 'Vertrag Nr. 2026-1184',
      counterparty: 'Versicherung Plus',
      domain: 'versicherung.example',
      kind: 'contract',
      status: 'done',
      nextStep: 'Vertragsunterlagen vollständig eingegangen',
      reason: 'Vertragsnummer 2026-1184 erkannt',
      keys: [{ label: 'Vertragsnr.', value: '2026-1184' }],
      events: [
        { id: 12, day: dayOffset(-4), label: 'Unterlagen erhalten', phase: 'done', messageId: versicherungId, direction: 'in' },
      ],
      attachments: [
        { id: 'att-vers-1', name: 'Police_2026-1184.pdf', sizeLabel: '640 KB', messageId: versicherungId },
      ],
      messageIds: versicherungId != null ? [versicherungId] : [],
    },
    {
      id: 6,
      title: 'Fotobuch Bestellung',
      counterparty: 'Fotobuch Studio',
      domain: 'fotobuch.example',
      kind: 'order',
      status: 'done',
      nextStep: 'Versandt & abgeschlossen',
      reason: 'Versandbestätigung ohne offene Rechnung abgeschlossen',
      keys: [{ label: 'Bestellung', value: 'FB-9021' }],
      events: [
        { id: 13, day: dayOffset(-8), label: 'Bestellt', phase: 'start', messageId: fotobuchId, direction: 'in' },
        { id: 14, day: dayOffset(-5), label: 'Versandt', phase: 'done', messageId: fotobuchId, direction: 'in' },
      ],
      attachments: [
        { id: 'att-fb-1', name: 'Rechnung_FB-9021.pdf', sizeLabel: '142 KB', messageId: fotobuchId },
      ],
      messageIds: fotobuchId != null ? [fotobuchId] : [],
    },
    {
      id: 7,
      title: 'Postfach-Dokumente',
      counterparty: 'Online-Banking',
      domain: 'bank.example',
      kind: 'invoice',
      status: 'done',
      nextStep: 'Abgerufen & erledigt',
      reason: 'Monatlicher Kontoauszug-Hinweis',
      keys: [{ label: 'Hinweis', value: 'Kontoauszug' }],
      events: [
        { id: 15, day: dayOffset(-7), label: 'Eingegangen', phase: 'done', messageId: bankId, direction: 'in' },
      ],
      messageIds: bankId != null ? [bankId] : [],
    },
  ]

  const securityIds = findIds(/Bestätigungscode|sign-in/)
  const allCategories: AsideCategory[] = [
    {
      id: 'cat-newsletter',
      label: 'Newsletter & Angebote',
      icon: 'tag',
      count: shopOrder != null ? 1 : 0,
      summary: 'Shop Wochenpost, Streamingdienst',
      messageIds: shopOrder != null ? [shopOrder] : [],
    },
    {
      id: 'cat-security',
      label: 'Sicherheit & Logins',
      icon: 'warning',
      count: securityIds.length,
      summary: 'Bestätigungscode, Neue Anmeldung',
      messageIds: securityIds,
    },
    {
      id: 'cat-social',
      label: 'Social Media & Hinweise',
      icon: 'mail',
      count: 0,
      summary: 'Wiederkehrende Benachrichtigungen',
      messageIds: [],
    },
  ]
  const categories = allCategories.filter((c) => c.count > 0)

  return { cases, categories }
}

/** A case as the backend sends it (`CaseView` in halcyon-core). */
export interface CaseView {
  id: number
  title: string
  counterparty: string
  domain: string | null
  kind: CaseKind
  status: CaseStatus
  next_step: string
  due_at: number | null
  due_label: string | null
  amount_cents: number | null
  currency: string | null
  reason: string
  created_at: number
  updated_at: number
  events: {
    id: number
    message_id: number | null
    occurred_at: number
    kind: string | null
    label: string
    phase: CaseEvent['phase']
    direction: 'in' | 'out' | null
  }[]
  keys: { label: string; value: string; normalized: string }[]
  message_ids: number[]
}

/** Keys used only for linking; not worth showing on the card. */
const HIDDEN_KEYS = new Set(['Verlauf', 'Artikel'])

function localDay(unixSeconds: number): string {
  const d = new Date(unixSeconds * 1000)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

function formatAmount(cents: number, currency: string | null): string {
  return new Intl.NumberFormat('de-DE', { style: 'currency', currency: currency ?? 'EUR' }).format(
    cents / 100,
  )
}

export function fromCaseView(v: CaseView): CaseItem {
  return {
    id: v.id,
    title: v.title,
    counterparty: v.counterparty,
    domain: v.domain,
    kind: v.kind,
    status: v.status,
    nextStep: v.next_step,
    dueLabel: v.due_label ?? undefined,
    amount: v.amount_cents != null ? formatAmount(v.amount_cents, v.currency) : undefined,
    reason: v.reason,
    keys: v.keys
      .filter((k) => !HIDDEN_KEYS.has(k.label))
      .map((k) => ({ label: k.label, value: k.value })),
    events: v.events.map((e) => ({
      id: e.id,
      day: localDay(e.occurred_at),
      label: e.label,
      phase: e.phase,
      messageId: e.message_id ?? undefined,
      direction: e.direction ?? undefined,
    })),
    messageIds: v.message_ids,
  }
}
