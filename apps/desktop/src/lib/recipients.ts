import type { Recipient } from './types'

const ADDRESS = /^[^\s@<>"]+@[^\s@<>"]+\.[^\s@<>"]+$/

/** `Name <a@b>` list as text; names with commas or quotes get quoted. */
export function formatRecipients(list: Recipient[]): string {
  return list
    .map((r) => {
      if (!r.name) return r.address
      const name = /[",;<>@]/.test(r.name) ? `"${r.name.replace(/"/g, '\\"')}"` : r.name
      return `${name} <${r.address}>`
    })
    .join(', ')
}

/**
 * Parses what the user typed: addresses separated by comma, semicolon or
 * line break, each either `a@b`, `<a@b>` or `Name <a@b>`. Commas inside
 * quotes or angle brackets do not split ("Muster, Max" <m@x>).
 */
export function parseRecipients(input: string): {
  recipients: Recipient[]
  invalid: string[]
} {
  const parts: string[] = []
  let current = ''
  let quoted = false
  let angle = false
  for (const ch of input) {
    if (ch === '"' && !angle) quoted = !quoted
    else if (ch === '<' && !quoted) angle = true
    else if (ch === '>' && !quoted) angle = false
    if (!quoted && !angle && (ch === ',' || ch === ';' || ch === '\n')) {
      parts.push(current)
      current = ''
    } else {
      current += ch
    }
  }
  parts.push(current)

  const recipients: Recipient[] = []
  const invalid: string[] = []
  const seen = new Set<string>()
  for (const raw of parts.map((p) => p.trim()).filter(Boolean)) {
    const m = raw.match(/^(.*?)\s*<([^<>]+)>$/)
    let name: string | null = null
    let address = raw
    if (m) {
      name =
        m[1]
          .trim()
          .replace(/^"(.*)"$/, '$1')
          .replace(/\\"/g, '"')
          .trim() || null
      address = m[2].trim()
    }
    address = address.toLowerCase()
    if (!ADDRESS.test(address)) {
      invalid.push(raw)
      continue
    }
    if (seen.has(address)) continue
    seen.add(address)
    recipients.push({ name, address })
  }
  return { recipients, invalid }
}
