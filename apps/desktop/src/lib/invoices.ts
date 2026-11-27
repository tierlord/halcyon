// Invoices ("Rechnungen"): types, API wrappers, mock data and formatting helpers.
// Kept in one file on purpose; the Tauri commands are the contract with the Rust side.
import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export type AccountKind = 'private' | 'business'
export type InvoiceEvidence = 'attachment' | 'mail' | 'portal' | 'none'

export interface InvoiceRecord {
  id: number
  account_id: number
  account_email: string
  kind: AccountKind
  kind_overridden: boolean
  issuer: string
  domain: string | null
  title: string
  invoice_number: string | null
  amount_cents: number | null
  currency: string | null
  issued_at: number
  due_at: number | null
  paid: boolean
  paid_source: 'auto' | 'user' | null
  paid_note: string | null
  evidence: InvoiceEvidence
  file_name: string | null
  portal_url: string | null
  portal_label: string | null
  message_id: number | null
  case_id: number | null
}

export interface AccountKindInfo {
  account_id: number
  email: string
  kind: AccountKind
}

export interface InvoiceExportReport {
  path: string
  exported: number
  without_proof: number
}

// VITE_HALCYON_MOCK=1 shows the built-in test data even inside the app,
// e.g. for a first run on a phone without any account.
const native = isTauri() && import.meta.env.VITE_HALCYON_MOCK !== '1'

// ---------------------------------------------------------------- mock data

const DAY = 86400

function mockData() {
  const now = Math.floor(Date.now() / 1000)
  const year = new Date().getFullYear()
  const at = (y: number, m: number, d: number) => Math.floor(new Date(y, m, d, 10).getTime() / 1000)
  let n = 0
  const inv = (p: Partial<InvoiceRecord>): InvoiceRecord => ({
    id: ++n,
    account_id: 1,
    account_email: 'mara@example.org',
    kind: 'private',
    kind_overridden: false,
    issuer: 'Stadtwerke Beispielstadt',
    domain: null,
    title: 'Rechnung',
    invoice_number: null,
    amount_cents: null,
    currency: 'EUR',
    issued_at: now,
    due_at: null,
    paid: false,
    paid_source: null,
    paid_note: null,
    evidence: 'attachment',
    file_name: null,
    portal_url: null,
    portal_label: null,
    message_id: null,
    case_id: null,
    ...p,
  })
  const biz = { account_id: 2, account_email: 'mara@werkstatt.example.com', kind: 'business' as const }
  return [
    inv({ issuer: 'Stadtwerke Beispielstadt', title: 'Strom, Abschlag November', invoice_number: 'SW-2026-1187', amount_cents: 8900, issued_at: now - 6 * DAY, due_at: now + 2 * DAY, evidence: 'attachment', file_name: 'SW-2026-1187.pdf', message_id: 101 }),
    inv({ issuer: 'Netzwerk Nord', title: 'Internet und Telefon', amount_cents: 3999, issued_at: now - 20 * DAY, due_at: now - 3 * DAY, evidence: 'portal', portal_url: 'https://kundencenter.example.net', portal_label: 'Zum Kundencenter', message_id: 102 }),
    inv({ ...biz, issuer: 'Atelier Lindenhof', title: 'Druckkosten Katalog', invoice_number: 'R-4471', amount_cents: 124050, issued_at: now - 9 * DAY, due_at: now + 12 * DAY, evidence: 'attachment', file_name: 'R-4471.pdf', message_id: 103 }),
    inv({ ...biz, issuer: 'Cloudhaus', title: 'Hosting, Oktober', amount_cents: 2490, issued_at: now - 15 * DAY, due_at: null, evidence: 'portal', portal_url: 'https://konto.example.com/rechnungen', portal_label: 'Rechnung im Portal abrufen', message_id: 104 }),
    inv({ ...biz, issuer: 'Bürobedarf Weiss', title: 'Papier und Toner', amount_cents: 18730, issued_at: now - 30 * DAY, due_at: now - 16 * DAY, evidence: 'none', message_id: 105 }),
    inv({ issuer: 'Reisebüro Sonnenweg', title: 'Anzahlung Herbstreise', amount_cents: 45000, issued_at: now - 40 * DAY, due_at: now - 26 * DAY, paid: true, paid_source: 'user', evidence: 'mail', message_id: 106 }),
    inv({ issuer: 'Versicherung Nordlicht', title: 'Hausrat, Jahresbeitrag', invoice_number: 'HV-88213', amount_cents: 12480, issued_at: at(year, 0, 8), due_at: at(year, 0, 31), paid: true, paid_source: 'auto', paid_note: 'Wird per Lastschrift eingezogen', evidence: 'attachment', file_name: 'HV-88213.pdf', message_id: 107 }),
    inv({ ...biz, issuer: 'Cloudhaus', title: 'Hosting, September', amount_cents: 2490, issued_at: at(year, 8, 14), paid: true, paid_source: 'auto', paid_note: 'Per Kreditkarte bezahlt', evidence: 'portal', portal_url: 'https://konto.example.com/rechnungen', portal_label: null, message_id: 108 }),
    inv({ issuer: 'Stadtwerke Beispielstadt', title: 'Strom, Jahresabrechnung', invoice_number: 'SW-2025-0911', amount_cents: 41210, issued_at: at(year - 1, 10, 12), due_at: at(year - 1, 10, 26), paid: true, paid_source: 'user', evidence: 'attachment', file_name: 'SW-2025-0911.pdf' }),
    inv({ ...biz, issuer: 'Atelier Lindenhof', title: 'Fotoshooting Produktbilder', invoice_number: 'R-3920', amount_cents: 89000, issued_at: at(year - 1, 10, 3), due_at: at(year - 1, 10, 17), paid: true, paid_source: 'user', evidence: 'attachment', file_name: 'R-3920.pdf' }),
    inv({ ...biz, issuer: 'Bürobedarf Weiss', title: 'Ordner und Etiketten', amount_cents: 4560, issued_at: at(year - 1, 5, 21), paid: true, paid_source: 'auto', paid_note: 'Per Lastschrift bezahlt', evidence: 'mail', message_id: null }),
    inv({ issuer: 'Reisebüro Sonnenweg', title: 'Sommerurlaub, Restzahlung', amount_cents: null, currency: null, issued_at: at(year - 1, 4, 9), paid: true, paid_source: 'user', evidence: 'none' }),
  ]
}

let mockInvoices: InvoiceRecord[] | null = null
let mockAccounts: AccountKindInfo[] = [
  { account_id: 1, email: 'mara@example.org', kind: 'private' },
  { account_id: 2, email: 'mara@werkstatt.example.com', kind: 'business' },
]
const mockListeners = new Set<() => void>()
const store = () => (mockInvoices ??= mockData())
const notify = () => mockListeners.forEach((f) => f())
const yearOf = (ts: number) => new Date(ts * 1000).getFullYear()
const accountKind = (id: number): AccountKind => mockAccounts.find((a) => a.account_id === id)?.kind ?? 'private'

// ---------------------------------------------------------------- API

export function invoices(year: number | null): Promise<InvoiceRecord[]> {
  if (native) return invoke('invoices', { year })
  return Promise.resolve(store().filter((i) => year === null || yearOf(i.issued_at) === year).map((i) => ({ ...i })))
}

export function invoiceYears(): Promise<number[]> {
  if (native) return invoke('invoice_years')
  return Promise.resolve([...new Set(store().map((i) => yearOf(i.issued_at)))].sort((a, b) => b - a))
}

export function setInvoicePaid(id: number, paid: boolean): Promise<void> {
  if (native) return invoke('set_invoice_paid', { id, paid })
  const i = store().find((x) => x.id === id)
  if (i) {
    i.paid = paid
    i.paid_source = paid ? 'user' : null
    i.paid_note = null
  }
  notify()
  return Promise.resolve()
}

export function setInvoiceKind(id: number, kind: AccountKind | null): Promise<void> {
  if (native) return invoke('set_invoice_kind', { id, kind })
  const i = store().find((x) => x.id === id)
  if (i) {
    i.kind = kind ?? accountKind(i.account_id)
    i.kind_overridden = kind !== null && kind !== accountKind(i.account_id)
  }
  notify()
  return Promise.resolve()
}

export function accountKinds(): Promise<AccountKindInfo[]> {
  if (native) return invoke('account_kinds')
  return Promise.resolve(mockAccounts.map((a) => ({ ...a })))
}

export function setAccountKind(accountId: number, kind: AccountKind): Promise<void> {
  if (native) return invoke('set_account_kind', { accountId, kind })
  mockAccounts = mockAccounts.map((a) => (a.account_id === accountId ? { ...a, kind } : a))
  for (const i of store()) if (i.account_id === accountId && !i.kind_overridden) i.kind = kind
  notify()
  return Promise.resolve()
}

/** Resolves with the path of the stored proof (the mail itself for `mail`). */
export function openInvoiceFile(id: number): Promise<string> {
  if (native) return invoke('open_invoice_file', { id })
  return Promise.resolve(`/home/mara/Halcyon/Rechnungen/${id}.pdf`)
}

export function exportInvoices(year: number): Promise<InvoiceExportReport> {
  if (native) return invoke('export_invoices', { year })
  const list = store().filter((i) => yearOf(i.issued_at) === year)
  return Promise.resolve({
    path: `/home/mara/Dokumente/Halcyon/Rechnungen ${year}`,
    exported: list.length,
    without_proof: list.filter((i) => i.evidence === 'portal' || i.evidence === 'none').length,
  })
}

export async function onInvoicesChanged(handler: () => void): Promise<() => void> {
  if (native) return listen('invoices-changed', () => handler())
  mockListeners.add(handler)
  return () => mockListeners.delete(handler)
}

// ---------------------------------------------------------------- formatting

const nfCache = new Map<string, Intl.NumberFormat>()
export function formatMoney(cents: number, currency: string | null): string {
  const cur = currency ?? 'EUR'
  let nf = nfCache.get(cur)
  if (!nf) {
    try {
      nf = new Intl.NumberFormat('de-DE', { style: 'currency', currency: cur })
    } catch {
      nf = new Intl.NumberFormat('de-DE', { style: 'currency', currency: 'EUR' })
    }
    nfCache.set(cur, nf)
  }
  return nf.format(cents / 100)
}

const dateFmt = new Intl.DateTimeFormat('de-DE', { day: 'numeric', month: 'long' })
const weekdayFmt = new Intl.DateTimeFormat('de-DE', { weekday: 'long' })
const monthFmt = new Intl.DateTimeFormat('de-DE', { month: 'long' })

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()

export function dayDiff(ts: number, now = Date.now()): number {
  return Math.round((startOfDay(new Date(ts * 1000)) - startOfDay(new Date(now))) / (DAY * 1000))
}

/** "fällig Mittwoch" / "seit 3 Tagen überfällig"; `overdue` drives the warning colour. */
export function dueText(due: number | null, now = Date.now()): { text: string; overdue: boolean } {
  if (due === null) return { text: 'ohne Fälligkeit', overdue: false }
  const d = dayDiff(due, now)
  const date = new Date(due * 1000)
  if (d < 0) return { text: d === -1 ? 'seit gestern überfällig' : `seit ${-d} Tagen überfällig`, overdue: true }
  if (d === 0) return { text: 'heute fällig', overdue: false }
  if (d === 1) return { text: 'morgen fällig', overdue: false }
  if (d < 7) return { text: `fällig ${weekdayFmt.format(date)}`, overdue: false }
  return { text: `fällig am ${dateFmt.format(date)}`, overdue: false }
}

export function monthName(ts: number): string {
  return monthFmt.format(new Date(ts * 1000))
}

export function monthKey(ts: number): string {
  const d = new Date(ts * 1000)
  return `${d.getFullYear()}-${String(d.getMonth()).padStart(2, '0')}`
}

export function issuedText(ts: number): string {
  return dateFmt.format(new Date(ts * 1000))
}

export function initials(name: string): string {
  const parts = name.split(/\s+/).filter(Boolean)
  return ((parts[0]?.[0] ?? '') + (parts[1]?.[0] ?? '')).toUpperCase()
}

/** Sums by currency, ignoring invoices without an amount. */
export function totalsByCurrency(list: InvoiceRecord[]): Map<string, number> {
  const m = new Map<string, number>()
  for (const i of list) {
    if (i.amount_cents === null) continue
    const c = i.currency ?? 'EUR'
    m.set(c, (m.get(c) ?? 0) + i.amount_cents)
  }
  return m
}

export function formatTotals(list: InvoiceRecord[]): string {
  const m = totalsByCurrency(list)
  if (m.size === 0) return '0,00 €'
  return [...m].map(([c, v]) => formatMoney(v, c)).join(' + ')
}
