// Where a link really leads, for the link prompt of the reading pane.
//
// The verdict for a whole mail comes from the core (`phishing.rs`), which
// also looks at the links. This is the same look at a single link, done in
// the front end because the prompt needs it at click time and in the browser
// preview, which has no core. Keep both in step: percent-encoded host, number
// host, `user@` part, script scheme, alphabet mix and `xn--` names.

import type { MessageSummary } from './types'

export type LinkIssue = 'script' | 'encoded_host' | 'ip_host' | 'userinfo' | 'homograph' | 'punycode'

export interface LinkFacts {
  /** The real target host: percent-decoded, lowercase, international names as `xn--`. */
  host: string
  issues: LinkIssue[]
}

/** Whether the mail is kept out of "Für dich", bundles and notifications. */
export function holdsBack(m: MessageSummary): boolean {
  const p = m.phishing
  if (p.level === 'phishing') return true
  return p.level === 'suspicious' && !p.acknowledged
}

export const ISSUE_TEXT: Record<LinkIssue, string> = {
  script: 'Der Link führt Programmcode aus, statt eine Webseite zu öffnen.',
  encoded_host: 'Der Servername ist mit kodierten Zeichen getarnt.',
  ip_host: 'Der Link führt auf eine Zahlenadresse statt auf einen Namen.',
  userinfo: 'Vor dem Servernamen steht ein Benutzername. So wird das echte Ziel getarnt.',
  homograph: 'Der Servername mischt Buchstaben verschiedener Alphabete, die wie andere aussehen.',
  punycode: 'Der Servername enthält Sonderzeichen (hier als „xn--“ geschrieben).',
}

/** Issues that have no innocent use; a plain international name is not one. */
export const isStrong = (i: LinkIssue) => i !== 'punycode'

// RFC 3492, enough to look at the letters behind an `xn--` label.
function punycodeDecode(input: string): string | null {
  const BASE = 36
  const T_MIN = 1
  const T_MAX = 26
  const SKEW = 38
  const DAMP = 700
  const cut = input.lastIndexOf('-')
  const out: number[] = [...(cut >= 0 ? input.slice(0, cut) : '')].map((c) => c.codePointAt(0)!)
  const rest = cut >= 0 ? input.slice(cut + 1) : input
  let n = 128
  let i = 0
  let bias = 72
  let pos = 0
  const digit = (c: string): number => {
    const code = c.charCodeAt(0)
    if (code >= 97 && code <= 122) return code - 97
    if (code >= 65 && code <= 90) return code - 65
    if (code >= 48 && code <= 57) return code - 22
    return -1
  }
  while (pos < rest.length) {
    const oldI = i
    let w = 1
    for (let k = BASE; ; k += BASE) {
      if (pos >= rest.length) return null
      const d = digit(rest[pos++])
      if (d < 0) return null
      i += d * w
      const t = k <= bias ? T_MIN : k >= bias + T_MAX ? T_MAX : k - bias
      if (d < t) break
      w *= BASE - t
    }
    const len = out.length + 1
    let delta = Math.floor((i - oldI) / (oldI === 0 ? DAMP : 2))
    delta += Math.floor(delta / len)
    let k2 = 0
    while (delta > ((BASE - T_MIN) * T_MAX) / 2) {
      delta = Math.floor(delta / (BASE - T_MIN))
      k2 += BASE
    }
    bias = k2 + Math.floor(((BASE - T_MIN + 1) * delta) / (delta + SKEW))
    n += Math.floor(i / len)
    i %= len
    if (n > 0x10ffff) return null
    out.splice(i, 0, n)
    i += 1
  }
  return String.fromCodePoint(...out)
}

/** Latin letters beside Cyrillic or Greek ones in one word: the look-alike trick. */
function mixesScripts(word: string): boolean {
  let latin = false
  let lookalike = false
  for (const ch of word) {
    if (/\p{Script=Latin}/u.test(ch)) latin = true
    else if (/\p{Script=Cyrillic}|\p{Script=Greek}/u.test(ch)) lookalike = true
  }
  return latin && lookalike
}

function percentDecode(s: string): { text: string; changed: boolean } {
  let changed = false
  const bytes: number[] = []
  for (let i = 0; i < s.length; i++) {
    if (s[i] === '%' && /^[0-9a-f]{2}$/i.test(s.slice(i + 1, i + 3))) {
      bytes.push(parseInt(s.slice(i + 1, i + 3), 16))
      changed = true
      i += 2
    } else {
      for (const b of new TextEncoder().encode(s[i])) bytes.push(b)
    }
  }
  return { text: new TextDecoder().decode(new Uint8Array(bytes)), changed }
}

function isNumericHost(host: string): boolean {
  const labels = host.split('.')
  const numeric = (l: string) => /^\d+$/.test(l) || /^0x[0-9a-f]+$/i.test(l)
  return (labels.length === 4 && labels.every(numeric)) || (labels.length === 1 && numeric(labels[0]))
}

/** Looks at where a link really leads; null for links that are no web links. */
export function analyzeLink(href: string): LinkFacts | null {
  // Browsers drop tabs, newlines and spaces inside a URL, so `java\tscript:` must not slip through.
  const url = href.replace(/[\s\p{Cc}­͏᠎​-‏‪-‮⁠-⁤⁦-⁩﻿]/gu, '')
  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(url)?.[1].toLowerCase()
  let rest: string
  if (scheme) {
    if (scheme === 'javascript' || scheme === 'data' || scheme === 'vbscript') {
      return { host: '', issues: ['script'] }
    }
    if (scheme !== 'http' && scheme !== 'https') return null
    rest = url.slice(scheme.length + 1)
  } else if (url.startsWith('//')) {
    rest = url
  } else {
    return null
  }
  // Browsers treat `\` like `/` in web URLs.
  const after = rest.replace(/^[/\\]+/, '')
  const end = after.search(/[/\\?#]/)
  const authority = end < 0 ? after : after.slice(0, end)
  const issues: LinkIssue[] = []
  const at = authority.lastIndexOf('@')
  const hostPort = at >= 0 ? authority.slice(at + 1) : authority
  if (at >= 0) issues.push('userinfo')
  let rawHost: string
  let ipv6 = false
  if (hostPort.startsWith('[')) {
    rawHost = hostPort.slice(1).split(']')[0]
    ipv6 = true
  } else {
    rawHost = hostPort.replace(/:\d*$/, '')
  }
  const decoded = percentDecode(rawHost)
  const host = decoded.text.replace(/\.$/, '').toLowerCase()
  if (decoded.changed) issues.push('encoded_host')
  if (ipv6 || isNumericHost(host)) issues.push('ip_host')
  let idn = false
  let homograph = false
  for (const label of host.split('.')) {
    let unicode: string
    if (label.startsWith('xn--')) {
      idn = true
      unicode = punycodeDecode(label.slice(4)) ?? ''
    } else if (/[^\x00-\x7f]/.test(label)) {
      idn = true
      unicode = label
    } else {
      continue
    }
    homograph ||= mixesScripts(unicode)
  }
  if (homograph) issues.push('homograph')
  else if (idn) issues.push('punycode')
  return { host, issues }
}

/** The address a link text pretends to be (`example.com/login`), if it looks like one. */
export function shownHost(text: string): string | null {
  const t = text.replace(/[­​-‏⁠﻿]/g, '').trim()
  const m = t.match(/^(?:https?:\/\/)?((?:[a-z0-9-]+\.)+[a-z]{2,})(?:[/:?#]\S*)?$/i)
  return m ? m[1].replace(/^www\./, '').toLowerCase() : null
}

/** What a click on a link needs to decide whether to ask first. */
export interface LinkCheck {
  /** Real target host, decoded; empty for script links. */
  host: string
  /** The address the link text claims, when it names a different one. */
  claimed: string | null
  issues: LinkIssue[]
}

/**
 * Looks at a link for the prompt: a text that names another host than the
 * target (percent-encoding decoded before comparing), and any obfuscation of
 * the target itself, whatever the text says.
 */
export function checkLink(href: string, text: string): LinkCheck | null {
  const facts = analyzeLink(href)
  if (!facts) return null
  const real = facts.host.replace(/^www\./, '')
  const shown = shownHost(text)
  const claimed = shown && real && shown !== real && !real.endsWith(`.${shown}`) ? shown : null
  return { host: facts.host, claimed, issues: facts.issues }
}
