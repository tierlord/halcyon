const time = new Intl.DateTimeFormat('de-DE', { hour: '2-digit', minute: '2-digit' })
const weekday = new Intl.DateTimeFormat('de-DE', { weekday: 'short' })
const dayMonth = new Intl.DateTimeFormat('de-DE', { day: 'numeric', month: 'short' })
const full = new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium' })
const long = new Intl.DateTimeFormat('de-DE', { dateStyle: 'full', timeStyle: 'short' })

/** Compact date for the message list: time today, weekday this week, … */
export function listDate(ts: number | null): string {
  if (ts == null) return ''
  const d = new Date(ts * 1000)
  const now = new Date()
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
  if (d.getTime() >= startOfToday) return time.format(d)
  if (d.getTime() >= startOfToday - 6 * 24 * 3600 * 1000) return weekday.format(d)
  if (d.getFullYear() === now.getFullYear()) return dayMonth.format(d)
  return full.format(d)
}

export function longDate(ts: number | null): string {
  return ts == null ? '' : long.format(new Date(ts * 1000))
}

const INITIAL_STOP_WORDS = new Set([
  'über',
  'via',
  'von',
  'by',
  'for',
  'für',
  'and',
  'und',
  'at',
  'bei',
  'mit',
  'with',
  'de',
  'van',
  'der',
  'die',
  'das',
])

/** "Max Muster" → "MM", "Sensonics über Kleinanzeigen" → "SK", "rechnung@telekom.de" → "R". */
export function initials(name: string | null, addr: string | null): string {
  const source = (name ?? addr ?? '?').trim()
  const words = source.split(/\s+/).filter((w) => /\p{L}/u.test(w))
  const significant = words.filter((w, i) => i === 0 || !INITIAL_STOP_WORDS.has(w.toLowerCase()))
  if (significant.length >= 2) return (significant[0][0] + significant[1][0]).toUpperCase()
  return (significant[0]?.[0] ?? source[0] ?? '?').toUpperCase()
}

/** Short label for an account, e.g. "example.org". */
export function accountLabel(email: string): string {
  return email.split('@')[1] ?? email
}
