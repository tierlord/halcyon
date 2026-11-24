<script lang="ts" module>
  import type { AccountInfo, FolderInfo } from './types'

  /** What was right-clicked, for the context menu. */
  export type SidebarTarget =
    | { kind: 'unified' }
    | { kind: 'account'; account: AccountInfo }
    | { kind: 'folder'; account: AccountInfo; folder: FolderInfo }
</script>

<script lang="ts">
  import { onMount } from 'svelte'
  import { cubicOut } from 'svelte/easing'
  import { fade, slide } from 'svelte/transition'
  import * as api from './api'
  import Icon from './Icon.svelte'
  import { accountLabel } from './format'
  import { getThemeMode, setThemeMode, type ThemeMode } from './theme.svelte'
  import type { FolderRole, Overview, View } from './types'


  let {
    overview,
    view,
    section = 'mail',
    needsYou = false,
    activeEmails = [],
    onselect,
    onselectsection,
    onshowcases,
    onmanageaccounts,
    oncontext,
  }: {
    overview: Overview | null
    view: View
    /** Top-level view: Heute, Rechnungen, or the mailbox views. */
    section?: 'today' | 'invoices' | 'mail'
    /** Shows a quiet dot on "Heute" when something waits for the user. */
    needsYou?: boolean
    /** Accounts being synced right now. */
    activeEmails?: string[]
    onselect: (v: View) => void
    onselectsection: (s: 'today' | 'invoices') => void
    onshowcases?: () => void
    onmanageaccounts: () => void
    /** Right-click or context-menu key on an item; x/y in window coordinates. */
    oncontext: (target: SidebarTarget, x: number, y: number) => void
  } = $props()

  // First the list grows, then the folders fade in one after another.
  const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  const EXPAND_MS = reduceMotion ? 0 : 180
  const STAGGER_MS = reduceMotion ? 0 : 28

  function contextMenu(target: SidebarTarget) {
    return (e: MouseEvent) => {
      e.preventDefault()
      oncontext(target, e.clientX, e.clientY)
    }
  }

  // Keyboard users: context-menu key or Shift+F10 open the same menu.
  function contextKey(target: SidebarTarget) {
    return (e: KeyboardEvent) => {
      if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
        e.preventDefault()
        const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
        oncontext(target, r.left + 24, r.bottom)
      }
    }
  }

  const inMail = $derived(section === 'mail')
  const isUnified = (role: FolderRole) => inMail && view.kind === 'unified' && view.role === role
  let expandedAccountId = $state<number | null>(null)
  let autoExpandedOnce = false
  let settingsOpen = $state(false)
  let autoMarkRead = $state(true)
  let autoMarkReadBusy = $state(false)
  let brandRowEl: HTMLDivElement | undefined = $state()

  onMount(() => {
    api
      .autoMarkRead()
      .then((v) => {
        autoMarkRead = v
      })
      .catch(() => {})
  })

  $effect(() => {
    const accounts = overview?.accounts ?? []
    if (!autoExpandedOnce && accounts.length === 1 && accounts[0].folders.length > 0) {
      expandedAccountId = accounts[0].id
      autoExpandedOnce = true
    }
  })

  async function toggleAutoMarkRead() {
    if (autoMarkReadBusy) return
    const next = !autoMarkRead
    autoMarkRead = next
    autoMarkReadBusy = true
    try {
      await api.setAutoMarkRead(next)
    } catch {
      autoMarkRead = !next
    } finally {
      autoMarkReadBusy = false
    }
  }

  const unifiedFolders: { role: FolderRole; label: string; icon: ReturnType<typeof folderIcon> }[] = [
    { role: 'inbox', label: 'Alle Posteingänge', icon: 'inbox' },
    { role: 'flagged', label: 'Markiert', icon: 'star' },
    { role: 'sent', label: 'Gesendet', icon: 'send' },
    { role: 'drafts', label: 'Entwürfe', icon: 'compose' },
    { role: 'archive', label: 'Archiv', icon: 'archive' },
    { role: 'trash', label: 'Papierkorb', icon: 'trash' },
  ]

  function onWindowPointerDown(e: PointerEvent) {
    if (!settingsOpen) return
    if (brandRowEl && !brandRowEl.contains(e.target as Node)) {
      settingsOpen = false
    }
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (settingsOpen && e.key === 'Escape') {
      settingsOpen = false
    }
  }

  const themeOptions: { value: ThemeMode; label: string }[] = [
    { value: 'light', label: 'Hell' },
    { value: 'dark', label: 'Dunkel' },
    { value: 'auto', label: 'Automatisch' },
  ]

  function folderIcon(role: FolderRole | null) {
    switch (role) {
      case 'inbox':
        return 'inbox' as const
      case 'sent':
        return 'send' as const
      case 'drafts':
        return 'compose' as const
      case 'trash':
        return 'trash' as const
      case 'junk':
        return 'warning' as const
      case 'archive':
        return 'archive' as const
      case 'flagged':
        return 'star' as const
      case 'all':
        return 'mail' as const
      default:
        return 'folder' as const
    }
  }

  function selectAccount(accountId: number) {
    if (expandedAccountId === accountId && view.kind === 'account' && view.accountId === accountId) {
      expandedAccountId = null
      return
    }
    expandedAccountId = accountId
    onselect({ kind: 'account', accountId, role: 'inbox' })
  }

  function selectFolder(accountId: number, folder: FolderInfo) {
    if (!folder.selectable) return
    expandedAccountId = accountId
    onselect({
      kind: 'folder',
      accountId,
      folderId: folder.id,
      folderName: folder.name,
      folderRole: folder.role,
    })
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydown={onWindowKeydown} />

<nav class="sidebar" aria-label="Ordner">
  <div class="brand-wrap" bind:this={brandRowEl}>
    <div class="brand">
      <img src="/halcyon.svg" alt="" width="28" height="28" />
      <span>Halcyon</span>
      <button
        type="button"
        class="settings-btn"
        class:open={settingsOpen}
        onclick={() => (settingsOpen = !settingsOpen)}
        aria-expanded={settingsOpen}
        aria-haspopup="true"
        title="Einstellungen"
        aria-label="Einstellungen"
      >
        <Icon name="settings" size={16} />
      </button>
    </div>

    {#if settingsOpen}
      <div class="settings-menu" role="dialog" aria-label="Einstellungen">
        <div class="settings-label">Thema</div>
        <div class="theme-switch" role="radiogroup" aria-label="Thema">
          {#each themeOptions as opt (opt.value)}
            <button
              type="button"
              role="radio"
              aria-checked={getThemeMode() === opt.value}
              class="theme-opt"
              class:active={getThemeMode() === opt.value}
              onclick={() => setThemeMode(opt.value)}
            >
              {opt.label}
            </button>
          {/each}
        </div>
        <div class="settings-sep"></div>
        <div class="settings-row">
          <div class="settings-row-text">
            <span class="settings-row-title" id="auto-read-label">Automatisch als gelesen markieren</span>
            <span class="settings-row-hint">Verarbeitete Mails in anderen Programmen nicht mehr als neu zeigen</span>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={autoMarkRead}
            aria-labelledby="auto-read-label"
            class="switch-btn"
            class:on={autoMarkRead}
            disabled={autoMarkReadBusy}
            onclick={toggleAutoMarkRead}
          >
            <span class="switch-thumb"></span>
          </button>
        </div>
      </div>
    {/if}
  </div>

  <ul class="section">
    <li>
      <button class="item" class:active={section === 'today'} onclick={() => onselectsection('today')}>
        <Icon name="mailopen" />
        <span class="label">Heute</span>
        {#if needsYou}<span class="quiet-dot" title="Etwas wartet auf dich" aria-label="Etwas wartet auf dich"></span>{/if}
      </button>
    </li>
    <li>
      <button class="item" class:active={section === 'invoices'} onclick={() => onselectsection('invoices')}>
        <Icon name="paperclip" />
        <span class="label">Rechnungen</span>
      </button>
    </li>
    {#if onshowcases}
      <li>
        <button class="item" onclick={onshowcases}>
          <Icon name="folder" />
          <span class="label">Vorgänge &amp; Quellen</span>
        </button>
      </li>
    {/if}
  </ul>

  <h2>Postfächer</h2>
  <ul class="section">
    {#each unifiedFolders as uf (uf.role)}
      <li>
        <button
          class="item"
          class:active={isUnified(uf.role)}
          onclick={() => {
            expandedAccountId = null
            onselect({ kind: 'unified', role: uf.role })
          }}
          oncontextmenu={uf.role === 'inbox' ? contextMenu({ kind: 'unified' }) : undefined}
          onkeydown={uf.role === 'inbox' ? contextKey({ kind: 'unified' }) : undefined}
        >
          <Icon name={uf.icon} />
          <span class="label">{uf.label}</span>
        </button>
      </li>
    {/each}
  </ul>

  <div class="section-header">
    <h2>Konten</h2>
    <button
      type="button"
      class="edit-btn"
      onclick={onmanageaccounts}
      title="Konten verwalten"
      aria-label="Konten verwalten"
    >
      <Icon name="pencil" size={14} />
    </button>
  </div>
  <ul class="section accounts-scroll">
    {#each overview?.accounts ?? [] as account (account.id)}
      {@const expanded = expandedAccountId === account.id}
      {@const hasFolders = account.folders.length > 0}
      <li>
        <button
          class="item"
          class:active={inMail && view.kind === 'account' && view.accountId === account.id}
          class:parent-active={inMail && view.kind === 'folder' && view.accountId === account.id}
          class:syncing={activeEmails.includes(account.email)}
          aria-busy={activeEmails.includes(account.email)}
          aria-expanded={hasFolders ? expanded : undefined}
          title={activeEmails.includes(account.email)
            ? `${account.email} – wird abgerufen …`
            : account.email}
          onclick={() => selectAccount(account.id)}
          oncontextmenu={contextMenu({ kind: 'account', account })}
          onkeydown={contextKey({ kind: 'account', account })}
        >
          {#if hasFolders}
            <span class="chevron"><Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={14} /></span>
          {/if}
          <Icon name="mail" />
          <span class="label">{account.display_name ?? accountLabel(account.email)}</span>
        </button>

        {#if expanded && hasFolders}
          <ul
            class="folders"
            aria-label={`Ordner von ${account.display_name ?? accountLabel(account.email)}`}
            transition:slide={{ duration: EXPAND_MS, easing: cubicOut }}
          >
            {#each account.folders as folder, i (folder.id)}
              {@const folderActive = inMail && view.kind === 'folder' && view.folderId === folder.id}
              <li in:fade={{ duration: 160, delay: EXPAND_MS * 0.6 + i * STAGGER_MS }}>
                {#if folder.selectable}
                  <button
                    class="item folder-item"
                    class:active={folderActive}
                    style:padding-left={`${8 + Math.min(folder.depth, 4) * 12}px`}
                    title={folder.path}
                    onclick={() => selectFolder(account.id, folder)}
                    oncontextmenu={contextMenu({ kind: 'folder', account, folder })}
                    onkeydown={contextKey({ kind: 'folder', account, folder })}
                  >
                    <Icon name={folderIcon(folder.role)} size={15} />
                    <span class="label">{folder.name}</span>
                  </button>
                {:else}
                  <div
                    class="item folder-item disabled"
                    style:padding-left={`${8 + Math.min(folder.depth, 4) * 12}px`}
                    title={folder.path}
                  >
                    <Icon name={folderIcon(folder.role)} size={15} />
                    <span class="label">{folder.name}</span>
                  </div>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </li>
    {:else}
      <li class="empty">
        Noch keine Konten.
        <button type="button" class="add-link" onclick={onmanageaccounts}>Konto hinzufügen</button>
      </li>
    {/each}
  </ul>

</nav>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    background: var(--sidebar-bg);
    color: var(--sidebar-text);
    border-right: 1px solid var(--sidebar-border);
    padding: 14px 10px 0;
    overflow: hidden;
    user-select: none;
  }
  .brand-wrap {
    position: relative;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 2px 8px 18px;
    font-weight: 650;
    font-size: 17px;
    letter-spacing: 0.01em;
  }
  .brand img {
    background: var(--sidebar-logo-bg);
    border-radius: 7px;
    padding: 2px;
  }
  .settings-btn {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 5px;
    border: 0;
    border-radius: var(--radius);
    background: none;
    color: var(--sidebar-text-muted);
  }
  .settings-btn:hover,
  .settings-btn.open {
    background: var(--sidebar-hover);
    color: var(--sidebar-text);
  }
  .settings-menu {
    position: absolute;
    top: 38px;
    left: 6px;
    right: 6px;
    z-index: 20;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-raised);
    color: var(--text);
    box-shadow: 0 8px 24px color-mix(in srgb, var(--pitch-black) 26%, transparent);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .settings-label {
    font-size: 11px;
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }
  .theme-switch {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 3px;
    padding: 3px;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--border);
  }
  .theme-opt {
    border: 0;
    background: none;
    border-radius: 6px;
    padding: 5px 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .theme-opt:hover {
    color: var(--text);
  }
  .theme-opt.active {
    background: var(--accent-fill);
    color: var(--on-accent-fill);
  }
  .settings-sep {
    height: 1px;
    background: var(--border);
    margin: 2px 0;
  }
  .settings-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .settings-row-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .settings-row-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    line-height: 1.3;
  }
  .settings-row-hint {
    font-size: 11px;
    color: var(--text-muted);
    line-height: 1.35;
  }
  .switch-btn {
    flex: none;
    position: relative;
    width: 34px;
    height: 20px;
    padding: 0;
    border: 0;
    border-radius: 99px;
    background: color-mix(in srgb, var(--text) 22%, transparent);
    transition: background-color 0.16s ease;
    cursor: pointer;
  }
  .switch-btn.on {
    background: var(--accent-fill);
  }
  .switch-thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #ffffff;
    box-shadow: 0 1px 3px color-mix(in srgb, var(--pitch-black) 28%, transparent);
    transition: left 0.16s ease;
  }
  .switch-btn.on .switch-thumb {
    left: 16px;
  }
  .section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: 18px 8px 6px;
  }
  .section-header h2 {
    margin: 0;
  }
  .edit-btn {
    opacity: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 3px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: var(--sidebar-text-muted);
    transition: opacity 0.12s ease;
  }
  .section-header:hover .edit-btn,
  .section-header:focus-within .edit-btn {
    opacity: 1;
  }
  .edit-btn:hover {
    background: var(--sidebar-hover);
    color: var(--sidebar-text);
  }
  .add-link {
    display: block;
    margin-top: 4px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--sidebar-accent-text);
    font-weight: 600;
    font-size: 13px;
  }
  h2 {
    margin: 18px 8px 6px;
    font-size: 11px;
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--sidebar-text-muted);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .section {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 8px;
    border: 0;
    border-radius: var(--radius);
    background: none;
    text-align: left;
    color: var(--sidebar-text);
  }
  button.item:hover {
    background: var(--sidebar-hover);
  }
  /* Sync in progress: a soft band of light runs through the row, in the
     logo's blue green. Short syncs (under ~0.4 s) stay invisible thanks to
     the delayed fade-in, so quick background checks do not flicker. */
  .item.syncing {
    position: relative;
    overflow: hidden;
    isolation: isolate;
  }
  .item.syncing::before {
    content: '';
    position: absolute;
    inset: 0;
    z-index: -1;
    border-radius: inherit;
    background: linear-gradient(
      100deg,
      transparent 20%,
      var(--sync-glow) 50%,
      transparent 80%
    );
    /* Three times the row width: at both ends of the cycle the band is
       completely outside the row, so the restart is invisible. */
    background-size: 300% 100%;
    background-position: 150% 0;
    opacity: 0;
    animation:
      sync-fade-in 0.4s ease-out 0.4s forwards,
      sync-sweep 3.2s linear infinite;
  }
  /* One calm pass, then a short pause before the next. */
  @keyframes sync-sweep {
    0% {
      background-position: 150% 0;
    }
    75%,
    100% {
      background-position: -50% 0;
    }
  }
  @keyframes sync-fade-in {
    to {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .item.syncing::before {
      animation: sync-fade-in 0.3s ease-out 0.4s forwards;
      background: color-mix(in srgb, var(--blue-green) 18%, transparent);
    }
  }
  .item.active {
    background: var(--sidebar-selection);
    box-shadow: inset 3px 0 0 var(--highlight);
    font-weight: 600;
  }
  .item.parent-active {
    font-weight: 600;
  }
  .chevron {
    display: inline-flex;
    align-items: center;
    margin-right: -4px;
    color: var(--sidebar-text-muted);
  }
  .chevron :global(svg) {
    color: var(--sidebar-text-muted);
  }
  .folders {
    list-style: none;
    margin: 2px 0 6px 14px;
    padding: 0 0 0 6px;
    border-left: 1px solid var(--sidebar-border);
  }
  .folder-item {
    gap: 8px;
    padding-top: 5px;
    padding-bottom: 5px;
    font-size: 13px;
  }
  .item :global(svg) {
    flex: none;
    color: var(--sidebar-accent-text);
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* A dot, not a number: something is waiting, nothing is counted. */
  .quiet-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
    background: var(--ochre);
  }
  .disabled {
    color: var(--sidebar-text-muted);
    cursor: default;
  }
  .disabled :global(svg) {
    color: var(--sidebar-text-muted);
  }
  .empty {
    padding: 6px 8px;
    color: var(--sidebar-text-muted);
    font-size: 13px;
  }
  .accounts-scroll {
    flex: 1 1 auto;
    min-height: 80px;
    padding-bottom: 14px;
    overflow-y: auto;
    scrollbar-width: thin;
  }
</style>
