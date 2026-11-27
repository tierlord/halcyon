import type { MessageSummary, MessageTurnInfo, SourceRecord } from './types'

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
  /** Machine kind from the linker, e.g. `order.shipped`; absent in mock data. */
  kind?: string
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
  amountCents?: number
  currency?: string
  orderDate?: string
  reason: string
  keys: CaseKey[]
  events: CaseEvent[]
  attachments?: CaseAttachment[]
  messageIds: number[]
  /** Unix seconds of the last change (status etc.); absent in mock data. */
  updatedAt?: number
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

  const stadtwerkeStrom = findId(/Abschlagsrechnung Strom/)
  const stadtwerkeJahr = findId(/Jahresabrechnung 2025/)
  const shopOrder = findId(/20 % auf alles/)
  const reiseId = findId(/Hamburg/)
  const fotobuchId = findId(/Fotobuch/)
  const versicherungId = findId(/Vertrag Nr\. 2026-1184/)
  const bankId = findId(/Neue Dokumente/)
  const dhlId = findId(/Ihre Sendung kommt heute/)
  const lieferungId = findId(/Lieferung KW 44/)
  const werkzeugId = findId(/Rechnung 2026-0417/)

  const cases: CaseItem[] = [
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
        { id: 14, day: dayOffset(-1), label: 'Zugestellt', kind: 'order.delivered', phase: 'done', messageId: fotobuchId, direction: 'in' },
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
    {
      id: 8,
      title: 'Bestellung 48213',
      counterparty: 'Pixelwerk',
      domain: 'pixelwerk.example',
      kind: 'order',
      status: 'waiting',
      nextStep: 'Unterwegs · wartet auf Zustellung',
      amount: '89,90 €',
      amountCents: 8990,
      currency: 'EUR',
      orderDate: dayOffset(-4),
      reason: 'Bestellbestätigung und Versandmail mit gleicher Bestellnummer',
      keys: [{ label: 'Bestellung', value: '48213' }],
      events: [
        { id: 20, day: dayOffset(-4), label: 'Bestellt', kind: 'order.placed', phase: 'start', messageId: dhlId, direction: 'in' },
        { id: 21, day: dayOffset(-2), label: 'Versandt', kind: 'order.shipped', phase: 'update', messageId: dhlId, direction: 'in' },
        { id: 22, day: dayOffset(0), label: 'In Zustellung', phase: 'update', messageId: dhlId, direction: 'in' },
      ],
      messageIds: dhlId != null ? [dhlId] : [],
    },
    {
      id: 9,
      title: 'Bestellung ZL-7730',
      counterparty: 'Zulieferer Kranz',
      domain: 'zulieferer.example',
      kind: 'order',
      status: 'waiting',
      nextStep: 'Unterwegs · wartet auf Zustellung',
      amount: '312,00 €',
      amountCents: 31200,
      currency: 'EUR',
      orderDate: dayOffset(-11),
      reason: 'Versandmail zur Bestellung ZL-7730',
      keys: [{ label: 'Bestellung', value: 'ZL-7730' }],
      events: [
        { id: 23, day: dayOffset(-11), label: 'Bestellt', kind: 'order.placed', phase: 'start', messageId: lieferungId, direction: 'in' },
        { id: 24, day: dayOffset(-7), label: 'Versandt', kind: 'order.shipped', phase: 'update', messageId: lieferungId, direction: 'in' },
      ],
      messageIds: lieferungId != null ? [lieferungId] : [],
    },
    {
      id: 10,
      title: 'Bestellung WB-2291',
      counterparty: 'Werkzeugbedarf',
      domain: 'muster-werkstatt.example',
      kind: 'order',
      status: 'waiting',
      nextStep: 'Bestellt · wartet auf Versand',
      amount: '57,40 €',
      amountCents: 5740,
      currency: 'EUR',
      orderDate: dayOffset(-9),
      reason: 'Bestellbestätigung erkannt',
      keys: [{ label: 'Bestellung', value: 'WB-2291' }],
      events: [
        { id: 25, day: dayOffset(-9), label: 'Bestellt', kind: 'order.placed', phase: 'start', messageId: werkzeugId, direction: 'in' },
      ],
      messageIds: werkzeugId != null ? [werkzeugId] : [],
    },
    {
      id: 11,
      title: 'Bestellung 48377',
      counterparty: 'Pixelwerk',
      domain: 'pixelwerk.example',
      kind: 'order',
      status: 'waiting',
      nextStep: 'Unterwegs · wartet auf Zustellung',
      amount: '24,90 €',
      amountCents: 2490,
      currency: 'EUR',
      orderDate: dayOffset(-3),
      reason: 'Versandmail zur Bestellung 48377',
      keys: [{ label: 'Bestellung', value: '48377' }],
      events: [
        { id: 26, day: dayOffset(-3), label: 'Bestellt', kind: 'order.placed', phase: 'start', messageId: dhlId, direction: 'in' },
        { id: 27, day: dayOffset(-1), label: 'Versandt', kind: 'order.shipped', phase: 'update', messageId: dhlId, direction: 'in' },
      ],
      messageIds: dhlId != null ? [dhlId] : [],
    },
    {
      id: 12,
      title: 'Bestellung 48402',
      counterparty: 'Pixelwerk',
      domain: 'pixelwerk.example',
      kind: 'order',
      status: 'waiting',
      nextStep: 'Bestellt · wartet auf Versand',
      amount: '129,00 €',
      amountCents: 12900,
      currency: 'EUR',
      orderDate: dayOffset(-1),
      reason: 'Bestellbestätigung erkannt',
      keys: [{ label: 'Bestellung', value: '48402' }],
      events: [
        { id: 28, day: dayOffset(-1), label: 'Bestellt', kind: 'order.placed', phase: 'start', messageId: lieferungId, direction: 'in' },
      ],
      messageIds: lieferungId != null ? [lieferungId] : [],
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
  const events = v.events.map((e) => ({
    id: e.id,
    day: localDay(e.occurred_at),
    label: e.label,
    kind: e.kind ?? undefined,
    phase: e.phase,
    messageId: e.message_id ?? undefined,
    direction: e.direction ?? undefined,
  }))
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
    amountCents: v.amount_cents ?? undefined,
    currency: v.currency ?? undefined,
    orderDate: events[0]?.day ?? localDay(v.created_at),
    reason: v.reason,
    keys: v.keys
      .filter((k) => !HIDDEN_KEYS.has(k.label))
      .map((k) => ({ label: k.label, value: k.value })),
    events,
    messageIds: v.message_ids,
    updatedAt: v.updated_at,
  }
}

/**
 * Sample sources for the browser preview (the backend computes them natively).
 * Matched to mock mail by sender domain, like the real source records.
 */
export function createMockSources(messages: MessageSummary[]): SourceRecord[] {
  const nowSec = Math.floor(Date.now() / 1000)
  const latest = (domain: string) =>
    messages
      .filter((m) => m.header.from_addr?.endsWith(`@${domain}`) && m.folder_role === 'inbox')
      .sort((a, b) => (b.header.date ?? 0) - (a.header.date ?? 0))[0]?.id ?? null
  const make = (
    id: number,
    display_name: string,
    domain: string,
    mode: SourceRecord['mode'],
    is_newsletter: boolean,
    anomaly: SourceRecord['anomaly'] = null,
  ): SourceRecord => ({
    id,
    source_key: `noreply@${domain}`,
    display_name,
    domain,
    mode,
    user_mode: false,
    total_messages: 40 + id * 7,
    read_messages: 12,
    dmarc_pass_count: 30,
    is_newsletter,
    first_seen_at: nowSec - 300 * 86400,
    last_seen_at: nowSec - 3600,
    median_interval_secs: 7 * 86400,
    median_amount_cents: null,
    amount_sample_count: 0,
    anomaly,
    patterns: [],
    latest_message_id: latest(domain),
  })
  return [
    make(1, 'Shop Wochenpost', 'shop.example', 'bundle', true),
    make(2, 'Streamingdienst', 'stream.example', 'mute', true),
    make(3, 'GitHub', 'github.example', 'bundle', false),
    make(4, 'Zahlfix', 'zahlfix.example', 'latest_only', false),
    make(5, 'Kleinmarkt', 'kleinmarkt.example', 'show', false),
    make(6, 'Mobilfunk Nord', 'mobilfunk-nord.example', 'show', false, {
      kind: 'unusual_amount',
      needs_you: true,
      reason: 'Die Mobilfunk-Rechnung ist diesmal 12 € höher als sonst',
    }),
  ]
}

/** Sample "who is on turn" results for the browser preview. */
export function createMockTurns(messages: MessageSummary[]): MessageTurnInfo[] {
  const find = (re: RegExp, sent = false) =>
    messages.find((m) => re.test(m.header.subject ?? '') && (m.folder_role === 'sent') === sent)
  const jonas = find(/Dienstplan Dezember/)
  const rennrad = find(/^Re: Nutzer-Anfrage/, true)
  const out: MessageTurnInfo[] = []
  if (jonas) {
    out.push({
      message_id: jonas.id,
      status: 'needs_you',
      follow_up_due: false,
      reason: 'Die letzte Mail kommt von Jonas, du stehst in An und sie enthält eine Frage',
      confidence: 0.8,
    })
  }
  if (rennrad) {
    out.push({
      message_id: rennrad.id,
      status: 'waiting',
      follow_up_due: true,
      reason: 'Deine Mail ist die letzte und enthält eine Frage',
      confidence: 0.7,
    })
  }
  return out
}
