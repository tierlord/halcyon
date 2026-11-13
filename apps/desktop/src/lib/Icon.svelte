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
    pencil: 'M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z',
    plus: 'M12 5v14M5 12h14',
    trash: 'M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3',
    check: 'M5 12.5l4.5 4.5L19 7.5',
    star: 'M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8-5.2-2.7-5.2 2.7 1-5.8-4.3-4.1 5.9-.9z',
    mailopen: 'M4 10l8-6 8 6v9H4zM4 10l8 5 8-5',
    send: 'M4 12l16-8-6 16-2.5-6.5zM11.5 13.5L20 4',
    reply: 'M9 7L4 12l5 5M4 12h10a6 6 0 0 1 6 6v1',
    replyall: 'M11 7l-5 5 5 5M7 7l-5 5 5 5M6 12h8a6 6 0 0 1 6 6v1',
    forward: 'M15 7l5 5-5 5M20 12H10a6 6 0 0 0-6 6v1',
    compose: 'M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z',
    paperclip: 'M21 11.5l-8.6 8.6a5 5 0 0 1-7.1-7.1l8.6-8.6a3.3 3.3 0 0 1 4.7 4.7l-8.6 8.6a1.7 1.7 0 0 1-2.4-2.4l7.9-7.9',
    image: 'M4 5h16v14H4zM4 16l5-5 4 4 2-2 5 5M15 9.5h.01',
    download: 'M12 4v11M7 10l5 5 5-5M5 20h14',
    warning: 'M12 3l10 18H2zM12 10v5M12 18h.01',
    folder: 'M3 6h6l2 2h10v11H3z',
    archive: 'M4 4h16v4H4zM6 8v11h12V8M10 12h4',
    chevronRight: 'M9 6l6 6-6 6',
    chevronDown: 'M6 9l6 6 6-6',
    settings: 'M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7zM19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9c.26.604.852.997 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z',
  } as const

  let {
    name,
    size = 18,
    filled = false,
  }: { name: keyof typeof paths; size?: number; filled?: boolean } = $props()
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill={filled ? 'currentColor' : 'none'}
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={paths[name]} />
</svg>
