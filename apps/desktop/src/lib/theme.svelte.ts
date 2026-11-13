export type ThemeMode = 'auto' | 'light' | 'dark'

const STORAGE_KEY = 'halcyon.theme'

function readStoredMode(): ThemeMode {
  try {
    const v = localStorage.getItem(STORAGE_KEY)
    if (v === 'light' || v === 'dark' || v === 'auto') return v
  } catch {
    // Ignore storage errors.
  }
  return 'auto'
}

function applyToDom(mode: ThemeMode) {
  if (mode === 'auto') {
    delete document.documentElement.dataset.theme
  } else {
    document.documentElement.dataset.theme = mode
  }
}

const initialMode = readStoredMode()
applyToDom(initialMode)
let mode = $state<ThemeMode>(initialMode)

const darkQuery = window.matchMedia('(prefers-color-scheme: dark)')
let systemDark = $state(darkQuery.matches)
darkQuery.addEventListener('change', () => {
  systemDark = darkQuery.matches
})

export function getThemeMode(): ThemeMode {
  return mode
}

export function setThemeMode(next: ThemeMode) {
  mode = next
  applyToDom(next)
  try {
    if (next === 'auto') {
      localStorage.removeItem(STORAGE_KEY)
    } else {
      localStorage.setItem(STORAGE_KEY, next)
    }
  } catch {
    // Ignore storage errors.
  }
}

export function isAppDark(): boolean {
  return mode === 'auto' ? systemDark : mode === 'dark'
}
