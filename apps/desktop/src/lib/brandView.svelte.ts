// Brand logo and colour lookup for components of "Heute" that are not
// Today.svelte itself. Call `useBrand()` once while a component initialises;
// it re-renders the caller when a logo arrives or its colour was sampled.

import { onMount } from 'svelte'
import { brandStyle, logoColor } from './brandColor'
import { extractBrandDomain, getBrandLogo, subscribeLogos } from './brandLogos'

export function useBrand() {
  let version = $state(0)
  onMount(() => subscribeLogos(() => (version += 1)))

  const logo = (email: string | null) => {
    void version
    return email ? getBrandLogo(email) : null
  }
  /** Inline `--brand*` properties (see brandColor.ts), to be used with the `.brand-scope` class. */
  const style = (email: string | null, name: string): string => {
    const l = logo(email)
    const sampled = l ? logoColor(l.dataUrl, () => (version += 1)) : null
    return brandStyle(sampled ?? l?.bg ?? null, extractBrandDomain(email) ?? name)
  }
  return { logo, style }
}
