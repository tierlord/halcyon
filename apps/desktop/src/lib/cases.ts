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

export type CaseStatus = 'needs_you' | 'waiting' | 'aside' | 'done'

export interface CaseEvent {
  id: string
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

export interface CaseItem {
  id: string
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
      id: 'case-kleinanzeigen',
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
        { id: 'e1', day: dayOffset(-2), label: 'Anzeige online', phase: 'start', messageId: kleinanzeigenIds[2], direction: 'in' },
        { id: 'e2', day: dayOffset(-1), label: 'Erste Anfrage', phase: 'update', messageId: kleinanzeigenIds[1], direction: 'in' },
        { id: 'e3', day: dayOffset(-1), label: 'Deine Antwort', phase: 'update', messageId: kleinanzeigenIds[3], direction: 'out' },
        { id: 'e4', day: dayOffset(0), label: 'Neue Rückfrage', phase: 'due', messageId: kleinanzeigenIds[0], direction: 'in' },
      ],
      messageIds: kleinanzeigenIds.filter((id): id is number => id != null),
    },
    {
      id: 'case-stadtwerke-jahr',
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
        { id: 'e5', day: dayOffset(-3), label: 'Rechnung gestellt', phase: 'start', messageId: stadtwerkeJahr ?? stadtwerkeStrom, direction: 'in' },
        { id: 'e6', day: dayOffset(2), label: 'Fällig', phase: 'due' },
      ],
      messageIds: [stadtwerkeJahr, stadtwerkeStrom].filter((id): id is number => id != null),
    },
    {
      id: 'case-kranz',
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
        { id: 'e7', day: dayOffset(-1), label: 'Angebot erhalten', phase: 'start', messageId: kranzIds[0], direction: 'in' },
        { id: 'e8', day: dayOffset(0), label: 'Freigabe gesendet', phase: 'update', messageId: kranzIds[1], direction: 'out' },
        { id: 'e9', day: dayOffset(3), label: 'Rückmeldung erwartet', phase: 'due' },
      ],
      messageIds: kranzIds,
    },
    {
      id: 'case-reise',
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
        { id: 'e10', day: dayOffset(-6), label: 'Gebucht', phase: 'start', messageId: reiseId, direction: 'in' },
        { id: 'e11', day: dayOffset(4), label: 'Ticket-Abruf', phase: 'due' },
      ],
      messageIds: reiseId != null ? [reiseId] : [],
    },
    {
      id: 'case-versicherung',
      title: 'Vertrag Nr. 2026-1184',
      counterparty: 'Versicherung Plus',
      domain: 'versicherung.example',
      kind: 'contract',
      status: 'done',
      nextStep: 'Vertragsunterlagen vollständig eingegangen',
      reason: 'Vertragsnummer 2026-1184 erkannt',
      keys: [{ label: 'Vertragsnr.', value: '2026-1184' }],
      events: [
        { id: 'e12', day: dayOffset(-4), label: 'Unterlagen erhalten', phase: 'done', messageId: versicherungId, direction: 'in' },
      ],
      messageIds: versicherungId != null ? [versicherungId] : [],
    },
    {
      id: 'case-fotobuch',
      title: 'Fotobuch Bestellung',
      counterparty: 'Fotobuch Studio',
      domain: 'fotobuch.example',
      kind: 'order',
      status: 'done',
      nextStep: 'Versandt & abgeschlossen',
      reason: 'Versandbestätigung ohne offene Rechnung abgeschlossen',
      keys: [{ label: 'Bestellung', value: 'FB-9021' }],
      events: [
        { id: 'e13', day: dayOffset(-8), label: 'Bestellt', phase: 'start', messageId: fotobuchId, direction: 'in' },
        { id: 'e14', day: dayOffset(-5), label: 'Versandt', phase: 'done', messageId: fotobuchId, direction: 'in' },
      ],
      messageIds: fotobuchId != null ? [fotobuchId] : [],
    },
    {
      id: 'case-bank',
      title: 'Postfach-Dokumente',
      counterparty: 'Online-Banking',
      domain: 'bank.example',
      kind: 'invoice',
      status: 'done',
      nextStep: 'Abgerufen & erledigt',
      reason: 'Monatlicher Kontoauszug-Hinweis',
      keys: [{ label: 'Hinweis', value: 'Kontoauszug' }],
      events: [
        { id: 'e15', day: dayOffset(-7), label: 'Eingegangen', phase: 'done', messageId: bankId, direction: 'in' },
      ],
      messageIds: bankId != null ? [bankId] : [],
    },
  ]

  const categories: AsideCategory[] = [
    {
      id: 'cat-newsletter',
      label: 'Newsletter & Angebote',
      icon: 'tag',
      count: 2,
      summary: 'Shop Wochenpost, Streamingdienst',
      messageIds: shopOrder != null ? [shopOrder] : [],
    },
    {
      id: 'cat-security',
      label: 'Sicherheit & Logins',
      icon: 'warning',
      count: 2,
      summary: 'Bestätigungscode, Neue Anmeldung',
      messageIds: findIds(/Bestätigungscode|sign-in/),
    },
    {
      id: 'cat-social',
      label: 'Social Media & Hinweise',
      icon: 'mail',
      count: 1,
      summary: 'Wiederkehrende Benachrichtigungen',
      messageIds: [],
    },
  ]

  return { cases, categories }
}
