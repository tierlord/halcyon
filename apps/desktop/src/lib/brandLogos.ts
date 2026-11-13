import { brandLogo, saveBrandLogoAppearance } from './api'
import type { BrandLogo } from './types'

const FREEMAIL_DOMAINS = new Set([
  'gmail.com',
  'googlemail.com',
  'outlook.com',
  'hotmail.com',
  'hotmail.de',
  'live.com',
  'live.de',
  'msn.com',
  'yahoo.com',
  'yahoo.de',
  'ymail.com',
  'icloud.com',
  'me.com',
  'mac.com',
  'gmx.de',
  'gmx.net',
  'gmx.at',
  'gmx.ch',
  'gmx.com',
  'web.de',
  't-online.de',
  'freenet.de',
  'posteo.de',
  'posteo.net',
  'mailbox.org',
  'proton.me',
  'protonmail.com',
  'pm.me',
  'tutanota.com',
  'tuta.io',
  'aol.com',
  'aol.de',
  'arcor.de',
  'vodafone.de',
  'o2online.de',
  '1und1.de',
  'ionos.de',
  'fastmail.com',
  'zoho.com',
  'hey.com',
])

const TWO_LEVEL_TLDS = new Set([
  'co.uk',
  'org.uk',
  'me.uk',
  'ac.uk',
  'com.au',
  'net.au',
  'org.au',
  'co.jp',
  'or.jp',
  'ne.jp',
  'com.br',
  'co.nz',
  'co.za',
  'co.in',
  'com.cn',
  'com.mx',
  'com.tr',
  'co.kr',
  'com.pl',
])

export interface LogoAppearance {
  dataUrl: string
  bg: string | null
  padded: boolean
  scale: number
}

/** Extracts the registrable domain from an email address, filtering out known freemail providers. */
export function extractBrandDomain(email: string | null | undefined): string | null {
  if (!email) return null
  const at = email.lastIndexOf('@')
  if (at < 0) return null
  let host = email.slice(at + 1).trim().toLowerCase()
  if (!host || host.includes(' ') || !host.includes('.')) return null

  const parts = host.split('.').filter(Boolean)
  if (parts.length < 2) return null

  let registrable: string
  if (parts.length >= 3) {
    const lastTwo = `${parts[parts.length - 2]}.${parts[parts.length - 1]}`
    registrable = TWO_LEVEL_TLDS.has(lastTwo)
      ? `${parts[parts.length - 3]}.${lastTwo}`
      : lastTwo
  } else {
    registrable = parts.join('.')
  }

  if (FREEMAIL_DOMAINS.has(registrable) || FREEMAIL_DOMAINS.has(host)) return null
  return registrable
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image()
    img.onload = () => resolve(img)
    img.onerror = () => reject(new Error('Failed to decode logo image'))
    img.src = src
  })
}

/**
 * Draws a logo onto an offscreen 48x48 canvas to detect opaque icons vs. transparent
 * SVGs/PNGs, compute a high-contrast background color, and auto-scale logos with wide margins.
 */
export async function analyzeLogoAppearance(
  dataUrl: string,
): Promise<{ bg: string | null; padded: boolean; scale: number }> {
  try {
    const img = await loadImage(dataUrl)
    const size = 48
    const canvas = document.createElement('canvas')
    canvas.width = size
    canvas.height = size
    const ctx = canvas.getContext('2d', { willReadFrequently: true })
    if (!ctx) return { bg: '#ffffff', padded: true, scale: 1 }

    ctx.clearRect(0, 0, size, size)
    ctx.drawImage(img, 0, 0, size, size)
    const { data } = ctx.getImageData(0, 0, size, size)

    // Check border pixels for opaque square icons (e.g. apple-touch-icon).
    let borderOpaque = 0
    let borderTotal = 0
    for (let i = 0; i < size; i++) {
      for (const [x, y] of [
        [i, 1],
        [i, size - 2],
        [1, i],
        [size - 2, i],
      ]) {
        borderTotal++
        if (data[(y * size + x) * 4 + 3] >= 200) borderOpaque++
      }
    }
    if (borderOpaque / borderTotal >= 0.72) {
      return { bg: null, padded: false, scale: 1 }
    }

    // Analyze visible pixels for luminance, average color, and bounding box.
    let weightSum = 0
    let rSum = 0
    let gSum = 0
    let bSum = 0
    let minX = size
    let maxX = 0
    let minY = size
    let maxY = 0

    for (let y = 0; y < size; y++) {
      for (let x = 0; x < size; x++) {
        const idx = (y * size + x) * 4
        const a = data[idx + 3]
        if (a < 40) continue
        if (x < minX) minX = x
        if (x > maxX) maxX = x
        if (y < minY) minY = y
        if (y > maxY) maxY = y

        const w = a / 255
        weightSum += w
        rSum += data[idx] * w
        gSum += data[idx + 1] * w
        bSum += data[idx + 2] * w
      }
    }

    if (weightSum === 0) {
      return { bg: '#ffffff', padded: true, scale: 1 }
    }

    const avgR = rSum / weightSum
    const avgG = gSum / weightSum
    const avgB = bSum / weightSum
    const luminance = (0.2126 * avgR + 0.7152 * avgG + 0.0722 * avgB) / 255

    let scale = 1
    if (maxX >= minX && maxY >= minY) {
      const coverage = Math.max(maxX - minX + 1, maxY - minY + 1) / size
      if (coverage > 0.15 && coverage < 0.74) {
        scale = Math.min(1.42, Math.max(1, 0.82 / coverage))
      }
    }

    const bg =
      luminance > 0.45
        ? `rgb(${Math.round(avgR * 0.16 + 14)}, ${Math.round(avgG * 0.16 + 16)}, ${Math.round(avgB * 0.16 + 18)})`
        : '#ffffff'

    return { bg, padded: true, scale: Number(scale.toFixed(2)) }
  } catch {
    return { bg: '#ffffff', padded: true, scale: 1 }
  }
}

const cache = new Map<string, LogoAppearance | null>()
const inflight = new Map<string, Promise<void>>()
const listeners = new Set<() => void>()

export function subscribeLogos(fn: () => void): () => void {
  listeners.add(fn)
  return () => listeners.delete(fn)
}

function notify() {
  for (const fn of listeners) fn()
}

async function fetchAndAnalyze(domain: string): Promise<void> {
  try {
    const raw: BrandLogo | null = await brandLogo(domain)
    if (!raw) {
      cache.set(domain, null)
      notify()
      return
    }

    if (raw.bg !== null || !raw.padded) {
      cache.set(domain, {
        dataUrl: raw.data_url,
        bg: raw.bg,
        padded: raw.padded,
        scale: raw.scale,
      })
      notify()
      return
    }

    const analyzed = await analyzeLogoAppearance(raw.data_url)
    cache.set(domain, {
      dataUrl: raw.data_url,
      bg: analyzed.bg,
      padded: analyzed.padded,
      scale: analyzed.scale,
    })
    notify()

    // Persist appearance so future sessions skip canvas analysis.
    void saveBrandLogoAppearance(domain, analyzed.bg, analyzed.padded, analyzed.scale)
  } catch {
    cache.set(domain, null)
    notify()
  }
}

/**
 * Returns the cached logo appearance for an email address, or `undefined` while
 * triggering background resolution. Returns `null` for freemail or domains without a logo.
 */
export function getBrandLogo(email: string | null | undefined): LogoAppearance | null | undefined {
  const domain = extractBrandDomain(email)
  if (!domain) return null
  if (cache.has(domain)) return cache.get(domain) ?? null

  if (!inflight.has(domain)) {
    const task = fetchAndAnalyze(domain).finally(() => inflight.delete(domain))
    inflight.set(domain, task)
  }
  return undefined
}
