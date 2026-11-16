// Brand colours for the start view: every card/row gets a friendly tint taken
// from the logo of the sender it is about. Pure functions, no DOM except the
// optional logo sampling at the bottom.
//
// The constants below mirror the tokens in app.css (--card-surface, --text,
// --brand-mix per theme; dark: 42% at the start of the gradient). The contrast guard needs the real numbers, so keep
// them in sync when the tokens change (design/palette.md, "Karten").

type RGB = [number, number, number]

const PITCH_BLACK: RGB = [0x15, 0x11, 0x0e]
const SNOW: RGB = [0xf7, 0xf0, 0xf0]
const BLUE_SLATE: RGB = [0x3b, 0x59, 0x66]

interface ThemeSpec {
  surface: RGB
  text: RGB
  /** Share of the brand colour at the strongest point of the gradient. */
  mix: number
}

const LIGHT: ThemeSpec = {
  // --card-surface = snow 40% over white, --brand-mix 14%
  surface: mixRgb(SNOW, [255, 255, 255], 0.4),
  text: PITCH_BLACK,
  mix: 0.14,
}

/** Dark cards start at this share of the (darkened) brand colour: --brand-mix in dark. */
const DARK_MIX = 0.42
const DARK: ThemeSpec = {
  // --card-surface (dark) = blue-slate 22% over (snow 6% over pitch-black)
  surface: mixRgb(BLUE_SLATE, mixRgb(SNOW, PITCH_BLACK, 0.06), 0.22),
  text: SNOW,
  mix: DARK_MIX,
}

/** Body text is >= 4.5:1; detail text uses 74% text strength (--text-soft). */
const MIN_CONTRAST = 4.5
const SOFT_TEXT = 0.74

export function mixRgb(a: RGB, b: RGB, shareOfA: number): RGB {
  return [0, 1, 2].map((i) => a[i] * shareOfA + b[i] * (1 - shareOfA)) as RGB
}

function channel(v: number): number {
  const c = v / 255
  return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4)
}

export function luminance(c: RGB): number {
  return 0.2126 * channel(c[0]) + 0.7152 * channel(c[1]) + 0.0722 * channel(c[2])
}

/** WCAG 2.x contrast ratio. */
export function contrast(a: RGB, b: RGB): number {
  const la = luminance(a)
  const lb = luminance(b)
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05)
}

/** Parses `#rgb`, `#rrggbb`, `rgb(r, g, b)` and `rgba(...)`; null otherwise. */
export function parseColor(input: string | null | undefined): RGB | null {
  if (!input) return null
  const s = input.trim().toLowerCase()
  let m = /^#([0-9a-f]{3})$/.exec(s)
  if (m) return [...m[1]].map((h) => parseInt(h + h, 16)) as RGB
  m = /^#([0-9a-f]{6})(?:[0-9a-f]{2})?$/.exec(s)
  if (m) return [0, 2, 4].map((i) => parseInt(m![1].slice(i, i + 2), 16)) as RGB
  m = /^rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)/.exec(s)
  if (m) return [Number(m[1]), Number(m[2]), Number(m[3])].map((v) => Math.min(255, v)) as RGB
  return null
}

export function toHsl([r, g, b]: RGB): [number, number, number] {
  const R = r / 255
  const G = g / 255
  const B = b / 255
  const max = Math.max(R, G, B)
  const min = Math.min(R, G, B)
  const l = (max + min) / 2
  const d = max - min
  if (d === 0) return [0, 0, l]
  const s = d / (1 - Math.abs(2 * l - 1))
  let h: number
  if (max === R) h = ((G - B) / d) % 6
  else if (max === G) h = (B - R) / d + 2
  else h = (R - G) / d + 4
  return [(h * 60 + 360) % 360, s, l]
}

export function fromHsl(h: number, s: number, l: number): RGB {
  const c = (1 - Math.abs(2 * l - 1)) * s
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1))
  const m = l - c / 2
  const [r, g, b] =
    h < 60 ? [c, x, 0] : h < 120 ? [x, c, 0] : h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : h < 300 ? [x, 0, c] : [c, 0, x]
  return [r + m, g + m, b + m].map((v) => Math.round(v * 255)) as RGB
}

function hex(c: RGB): string {
  return '#' + c.map((v) => v.toString(16).padStart(2, '0')).join('')
}

/** Greys, near-white and near-black carry no usable hue. */
function isUsable([, s, l]: [number, number, number]): boolean {
  return s >= 0.18 && l >= 0.12 && l <= 0.9
}

function hash(key: string): number {
  let h = 2166136261
  for (let i = 0; i < key.length; i++) {
    h ^= key.charCodeAt(i)
    h = Math.imul(h, 16777619)
  }
  return h >>> 0
}

/**
 * Stable fallback colour for a sender without a usable brand colour. The hue
 * range skips the orange/brown band (15-55 degrees), which looks muddy on dark
 * surfaces.
 */
export function derivedColor(key: string): RGB {
  const hue = (55 + (hash(key.toLowerCase()) % 305)) % 360
  return fromHsl(hue, 0.55, 0.5)
}

/** Orange/brown hues turn muddy when mixed into a dark surface: keep them subtle. */
function isWarmBand(hue: number): boolean {
  return hue >= 15 && hue <= 55
}

/**
 * Pulls a harsh brand colour into a friendly range (saturation 0.35-0.7,
 * lightness 0.42-0.58) so it works as a tint source in both themes.
 */
export function normalizeBrand(c: RGB): RGB {
  const [h, s, l] = toHsl(c)
  return fromHsl(h, Math.min(0.7, Math.max(0.35, s)), Math.min(0.58, Math.max(0.42, l)))
}

function readable(source: RGB, t: ThemeSpec, k: number): boolean {
  const tinted = mixRgb(source, t.surface, t.mix * k)
  const soft = mixRgb(t.text, tinted, SOFT_TEXT)
  // The dark tint is sized right at the limit, so keep a small margin for the
  // browser's own colour rounding.
  const need = t === DARK ? MIN_CONTRAST + 0.15 : MIN_CONTRAST
  return contrast(t.text, tinted) >= need && contrast(soft, tinted) >= need
}

/**
 * Largest tint strength (0..1, multiplier for --brand-mix) at which body and
 * soft text still reach 4.5:1 on the light theme. 0 means: use the neutral surface.
 */
export function tintStrength(brand: RGB, start = 1): number {
  for (let k = start; k >= 0.3; k -= 0.15) {
    if (readable(brand, LIGHT, k)) return Number(k.toFixed(2))
  }
  return 0
}

/**
 * Dark theme: a saturated version of the brand hue that is as light as the
 * contrast guard allows at the strongest point of the gradient (DARK_MIX), so
 * snow-coloured text stays >= 4.5:1 (body and soft) while the card still reads
 * as clearly coloured. Per hue, because blue tolerates far more lightness
 * than yellow-green.
 */
export function darkTint(hue: number): RGB {
  for (let l = 0.52; l >= 0.16; l -= 0.02) {
    const c = fromHsl(hue, 0.82, l)
    if (readable(c, DARK, 1)) return c
  }
  return fromHsl(hue, 0.82, 0.16)
}

/**
 * Dark theme accent (progress bars, chips, dots, edge, filled button): more
 * saturated and lighter than the light brand. Light enough for the pitch-black
 * label of the filled button to reach 4.5:1.
 */
export function darkAccent(brand: RGB): RGB {
  const [h, s] = toHsl(brand)
  const sat = Math.min(0.88, Math.max(0.6, s + 0.25))
  for (let l = 0.6; l <= 0.78; l += 0.02) {
    const c = fromHsl(h, sat, l)
    if (contrast(PITCH_BLACK, c) >= MIN_CONTRAST) return c
  }
  return fromHsl(h, sat, 0.78)
}

/** Label colour for a filled brand button, or null when neither dark nor light text reaches 4.5:1. */
export function buttonLabel(brand: RGB): RGB | null {
  const best = contrast(PITCH_BLACK, brand) >= contrast(SNOW, brand) ? PITCH_BLACK : SNOW
  return contrast(best, brand) >= MIN_CONTRAST ? best : null
}

/**
 * Inline custom properties for a card: --brand (light accent and tint source),
 * --brand-k (light tint strength), --brand-d (dark accent), --brand-dt (dark
 * tint source) and --brand-kd (dark tint strength). --brand-btn/--brand-on
 * exist when the filled light button is readable. The card CSS picks the set
 * for the current theme (see app.css, `.brand-scope`). `source` is a CSS
 * colour from the logo; `key` (domain or name) makes the fallback stable.
 */
export function brandStyle(source: string | null | undefined, key: string): string {
  const parsed = parseColor(source)
  const usable = parsed && isUsable(toHsl(parsed)) ? parsed : null
  const brand = normalizeBrand(usable ?? derivedColor(key))
  const [hue] = toHsl(brand)
  const warm = isWarmBand(hue)
  const k = tintStrength(brand, warm ? 0.55 : 1)
  // Warm hues turn brown when darkened; a slightly weaker tint keeps them amber.
  const kd = warm ? 0.75 : 1
  let css =
    `--brand:${hex(brand)};--brand-k:${k};--brand-d:${hex(darkAccent(brand))};` +
    `--brand-dt:${hex(darkTint(hue))};--brand-kd:${kd}`
  const label = buttonLabel(brand)
  if (label) css += `;--brand-btn:${hex(brand)};--brand-on:${hex(label)}`
  return css
}

// --- Sampling the dominant colour of a logo ---------------------------------

const sampled = new Map<string, string | null>()
const pending = new Set<string>()

/**
 * Dominant saturated colour of a logo image (data URL), cached. Returns
 * `undefined` while sampling runs (calls `onReady` afterwards), `null` when the
 * logo has no saturated colour (monochrome logos).
 */
export function logoColor(dataUrl: string, onReady: () => void): string | null | undefined {
  if (sampled.has(dataUrl)) return sampled.get(dataUrl)
  if (!pending.has(dataUrl)) {
    pending.add(dataUrl)
    const img = new Image()
    const done = (v: string | null) => {
      pending.delete(dataUrl)
      sampled.set(dataUrl, v)
      onReady()
    }
    img.onload = () => done(sample(img))
    img.onerror = () => done(null)
    img.src = dataUrl
  }
  return undefined
}

function sample(img: HTMLImageElement): string | null {
  try {
    const size = 32
    const canvas = document.createElement('canvas')
    canvas.width = size
    canvas.height = size
    const ctx = canvas.getContext('2d', { willReadFrequently: true })
    if (!ctx) return null
    ctx.drawImage(img, 0, 0, size, size)
    const data = ctx.getImageData(0, 0, size, size).data
    // 12 hue buckets of 30 degrees, weighted by saturation and mid-lightness.
    const buckets = Array.from({ length: 12 }, () => ({ w: 0, rgb: [0, 0, 0] as RGB }))
    let total = 0
    for (let i = 0; i < data.length; i += 4) {
      if (data[i + 3] < 128) continue
      const c: RGB = [data[i], data[i + 1], data[i + 2]]
      const [h, s, l] = toHsl(c)
      total += 1
      if (!isUsable([h, s, l])) continue
      const w = s * (1 - Math.abs(2 * l - 1))
      const b = buckets[Math.floor(h / 30) % 12]
      b.w += w
      for (let j = 0; j < 3; j++) b.rgb[j] += c[j] * w
    }
    const top = buckets.reduce((a, b) => (b.w > a.w ? b : a))
    // Too little colour in the logo (mostly grey/white): let the caller fall back.
    if (total === 0 || top.w / total < 0.03) return null
    return hex(top.rgb.map((v) => Math.round(v / top.w)) as RGB)
  } catch {
    return null
  }
}
