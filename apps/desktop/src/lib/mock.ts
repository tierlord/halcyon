// Made-up sample data for developing the UI in a browser.

import type {
  Account,
  AccountInfo,
  DiscoverResult,
  MessageSummary,
  Overview,
  ServerConfig,
  SyncUpdate,
} from './types'

const accounts: AccountInfo[] = [
  { id: 1, email: 'max@mail.example.org', display_name: null, unread_inbox: 0 },
  { id: 2, email: 'max@werkstatt.example.com', display_name: null, unread_inbox: 0 },
  { id: 3, email: 'max.muster@example.net', display_name: null, unread_inbox: 0 },
]

const fullAccounts: Account[] = [
  {
    id: 1,
    email: 'max@mail.example.org',
    display_name: null,
    imap: {
      host: 'imap.example.org',
      port: 993,
      security: 'tls',
      username: 'max@mail.example.org',
      auth: 'password',
    },
    smtp: {
      host: 'smtp.example.org',
      port: 465,
      security: 'tls',
      username: 'max@mail.example.org',
      auth: 'password',
    },
  },
  {
    id: 2,
    email: 'max@werkstatt.example.com',
    display_name: 'Werkstatt',
    imap: {
      host: 'mail.werkstatt.example.com',
      port: 993,
      security: 'tls',
      username: 'max@werkstatt.example.com',
      auth: 'password',
    },
    smtp: null,
  },
  {
    id: 3,
    email: 'max.muster@example.net',
    display_name: null,
    imap: {
      host: 'imap.example.net',
      port: 993,
      security: 'tls',
      username: 'max.muster@example.net',
      auth: 'password',
    },
    smtp: null,
  },
]

const now = Math.floor(Date.now() / 1000)
const hour = 3600
const day = 24 * hour

let nextId = 1
function msg(
  accountId: number,
  from: [string | null, string],
  subject: string,
  ago: number,
  opts: { seen?: boolean; list?: boolean; to?: string[] } = {},
): MessageSummary {
  const account = accounts.find((a) => a.id === accountId)!
  const id = nextId++
  return {
    id,
    account_id: accountId,
    account_email: account.email,
    folder_id: accountId * 10,
    folder_path: 'INBOX',
    header: {
      uid: id,
      message_id: `${id}@example.invalid`,
      in_reply_to: null,
      references: [],
      subject,
      from_name: from[0],
      from_addr: from[1],
      reply_to: null,
      to: opts.to ?? [account.email],
      cc: [],
      date: now - ago,
      size: 12_000,
      flags: opts.seen ? ['\\Seen'] : [],
      list_id: opts.list ? 'news.example.invalid' : null,
      list_unsubscribe: opts.list ? ['https://example.invalid/unsubscribe'] : [],
      list_unsubscribe_post: !!opts.list,
    },
  }
}

const all: MessageSummary[] = [
  msg(1, ['Telekom', 'rechnung@telekom.example'], 'Ihre Rechnung für November 2026', 25 * 60),
  msg(3, ['Kleinanzeigen', 'noreply@kleinanzeigen.example'], 'Nutzer-Anfrage zu deiner Anzeige "Rennrad 28 Zoll"', 2 * hour),
  msg(3, ['Kleinanzeigen', 'noreply@kleinanzeigen.example'], 'Neue Nachricht zu "Rennrad 28 Zoll"', 3 * hour, { seen: true }),
  msg(2, ['Sabine Kranz', 'kranz@zulieferer.example'], 'Angebot Ersatzteile – bitte bis Freitag bestätigen', 5 * hour),
  msg(1, ['PayPal', 'service@paypal.example'], 'Dein Bestätigungscode: 482913', 6 * hour, { seen: true }),
  msg(3, ['DHL Paket', 'noreply@dhl.example'], 'Ihre Sendung kommt heute zwischen 14:00 und 16:00', 8 * hour),
  msg(1, ['Stadtwerke', 'kundenservice@stadtwerke.example'], 'Abschlagsrechnung Strom', 1 * day + 2 * hour, { seen: true }),
  msg(3, ['Shop Wochenpost', 'news@shop.example'], 'Nur heute: 20 % auf alles ☀', 1 * day + 5 * hour, { list: true }),
  msg(2, ['Jonas Becker', 'jonas@muster-werkstatt.example'], 'Re: Dienstplan Dezember', 2 * day, { seen: true }),
  msg(3, ['GitHub', 'noreply@github.example'], '[halcyon] New sign-in from Linux', 3 * day, { seen: true }),
  msg(1, ['Versicherung Plus', 'vertrag@versicherung.example'], 'Ihr Vertrag Nr. 2026-1184 – Vertragsunterlagen', 4 * day, { seen: true }),
  msg(3, ['Reiseportal', 'buchung@reise.example'], 'Buchungsbestätigung: Hamburg, 12.–14. Dezember', 6 * day, { seen: true }),
  msg(2, [null, 'buchhaltung@muster-werkstatt.example'], 'Rechnung 2026-0417 Werkzeugbedarf', 9 * day, { seen: true }),
  msg(3, ['Streamingdienst', 'info@stream.example'], 'Neu diesen Monat', 12 * day, { seen: true, list: true }),
  msg(1, ['Online-Banking', 'hinweis@bank.example'], 'Neue Dokumente in Ihrem Postfach', 20 * day, { seen: true }),
  msg(3, ['Kleinanzeigen', 'noreply@kleinanzeigen.example'], 'Deine Anzeige "Rennrad 28 Zoll" ist online', 26 * day, { seen: true }),
  msg(2, ['Sabine Kranz', 'kranz@zulieferer.example'], 'Lieferung KW 44', 40 * day, { seen: true }),
  msg(1, ['Telekom', 'rechnung@telekom.example'], 'Ihre Rechnung für Oktober 2026', 31 * day, { seen: true }),
  msg(3, ['Fotobuch Studio', 'hallo@fotobuch.example'], 'Dein Fotobuch wurde versendet', 400 * day, { seen: true }),
]

const delay = <T>(value: T, ms = 120) => new Promise<T>((r) => setTimeout(() => r(value), ms))

const unread = (accountId: number | null) =>
  all.filter((m) => (accountId === null || m.account_id === accountId) && !m.header.flags.includes('\\Seen')).length

export function overview(): Promise<Overview> {
  return delay({
    accounts: accounts.map((a) => ({ ...a, unread_inbox: unread(a.id) })),
    unread_inbox: unread(null),
  })
}

export function messages(accountId: number | null, role: string): Promise<MessageSummary[]> {
  if (role !== 'inbox') return delay([])
  return delay(all.filter((m) => accountId === null || m.account_id === accountId))
}

export function search(query: string): Promise<MessageSummary[]> {
  const fold = (s: string) => s.normalize('NFD').replace(/\p{Diacritic}/gu, '').toLowerCase()
  const words = fold(query).split(/\s+/).filter(Boolean)
  return delay(
    all.filter((m) => {
      const hay = fold(
        [m.header.subject, m.header.from_name, m.header.from_addr, ...m.header.to].join(' '),
      )
      return words.every((w) => hay.split(/[^\p{L}\p{N}]+/u).some((t) => t.startsWith(w)))
    }),
  )
}

type Handler = (u: SyncUpdate) => void
const handlers = new Set<Handler>()

export function onSync(handler: Handler): () => void {
  handlers.add(handler)
  return () => handlers.delete(handler)
}

export async function startSync(): Promise<boolean> {
  const emit = (u: SyncUpdate) => handlers.forEach((h) => h(u))
  emit({ kind: 'started' })
  for (const a of accounts) {
    emit({ kind: 'account', email: a.email })
    await delay(null, 500)
  }
  emit({ kind: 'finished', new_messages: 0 })
  return true
}

export function listAccounts(): Promise<Account[]> {
  return delay(fullAccounts.map((a) => ({ ...a, imap: { ...a.imap }, smtp: a.smtp ? { ...a.smtp } : null })))
}

export function discoverAccount(email: string): Promise<DiscoverResult> {
  const domain = email.split('@')[1]?.trim().toLowerCase() || 'example.org'
  return delay(
    {
      candidates: [
        {
          source: 'ispdb',
          provider_name: domain,
          imap: {
            host: `imap.${domain}`,
            port: 993,
            security: 'tls',
            username: email.trim(),
            auth: 'password',
          },
          smtp: {
            host: `smtp.${domain}`,
            port: 465,
            security: 'tls',
            username: email.trim(),
            auth: 'password',
          },
        },
      ],
      // Addresses at "manuell.example" simulate a provider without autoconfig.
      reachable_index: domain.startsWith('manuell') ? null : 0,
      probe_error: domain.startsWith('manuell') ? 'Zeitüberschreitung' : null,
    },
    300,
  )
}

export function checkServer(server: ServerConfig): Promise<string> {
  // Lets the "manual settings" path be tried in the browser: hosts containing
  // "falsch" fail like an unreachable server.
  if (!server.host.trim() || server.host.includes('falsch')) {
    return new Promise((_, reject) =>
      setTimeout(() => reject(`${server.host || 'Server'} ist nicht erreichbar.`), 250),
    )
  }
  return delay(`${server.host} ist erreichbar und sicher verbunden.`, 250)
}

export function saveAccount(account: Account): Promise<Account> {
  if (account.id === 0) {
    const id = Math.max(0, ...fullAccounts.map((a) => a.id)) + 1
    const saved = { ...account, id }
    fullAccounts.push(saved)
    accounts.push({
      id,
      email: saved.email,
      display_name: saved.display_name,
      unread_inbox: 0,
    })
    return delay(saved, 200)
  }
  const idx = fullAccounts.findIndex((a) => a.id === account.id)
  if (idx >= 0) fullAccounts[idx] = account
  const infoIdx = accounts.findIndex((a) => a.id === account.id)
  if (infoIdx >= 0) {
    accounts[infoIdx] = {
      ...accounts[infoIdx],
      email: account.email,
      display_name: account.display_name,
    }
  }
  return delay(account, 200)
}

export function deleteAccount(id: number): Promise<void> {
  const idx = fullAccounts.findIndex((a) => a.id === id)
  if (idx >= 0) fullAccounts.splice(idx, 1)
  const infoIdx = accounts.findIndex((a) => a.id === id)
  if (infoIdx >= 0) accounts.splice(infoIdx, 1)
  return delay(undefined, 150)
}
