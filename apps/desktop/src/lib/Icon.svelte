<script lang="ts">
  // Small stroke icons (24×24 grid), so we need no icon font.
  const paths = {
    inbox: 'M3 13h5l2 3h4l2-3h5M5 5h14l2 8v6H3v-6z',
    sync: 'M20 11a8 8 0 0 0-14.3-4.9L4 8M4 4v4h4M4 13a8 8 0 0 0 14.3 4.9L20 16M20 20v-4h-4',
    search: 'M11 18a7 7 0 1 1 0-14 7 7 0 0 1 0 14zM21 21l-5-5',
    tag: 'M3 12V4h8l10 10-8 8zM7.5 8.5h.01',
    user: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 21a8 8 0 0 1 16 0',
    x: 'M6 6l12 12M18 6L6 18',
    unsubscribe: 'M4 6h16v12H4zM4 7l8 6 8-6M15 19l6-6',
    mail: 'M4 6h16v12H4zM4 7l8 6 8-6',
  } as const

  let { name, size = 18 }: { name: keyof typeof paths; size?: number } = $props()
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={paths[name]} />
</svg>
