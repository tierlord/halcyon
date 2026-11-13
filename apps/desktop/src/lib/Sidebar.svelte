<script lang="ts" module>
  import type { AccountInfo, FolderInfo } from './types'

  /** What was right-clicked, for the context menu. */
  export type SidebarTarget =
    | { kind: 'unified' }
    | { kind: 'account'; account: AccountInfo }
    | { kind: 'folder'; account: AccountInfo; folder: FolderInfo }
</script>

<script lang="ts">
  import { cubicOut } from 'svelte/easing'
  import { fade, slide } from 'svelte/transition'
  import Icon from './Icon.svelte'
  import { accountLabel } from './format'
  import type { CaseItem } from './cases'
  import { getThemeMode, setThemeMode, type ThemeMode } from './theme.svelte'
  import type { FolderRole, Overview, View } from './types'


  let {
    overview,
    view,
    activeEmails = [],
    cases = [],
    selectedCaseId = null,
    onselect,
    onmanageaccounts,
    oncontext,
    onshowcases,
    onselectcase,
    onhovercase,
  }: {
    overview: Overview | null
    view: View
    /** Accounts being synced right now. */
    activeEmails?: string[]
    cases?: CaseItem[]
    selectedCaseId?: number | null
    onselect: (v: View) => void
    onmanageaccounts: () => void
    /** Right-click or context-menu key on an item; x/y in window coordinates. */
    oncontext: (target: SidebarTarget, x: number, y: number) => void
    onshowcases?: () => void
    onselectcase?: (id: number) => void
    onhovercase?: (id: number | null) => void
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

  const isUnified = $derived(view.kind === 'unified')
  let expandedAccountId = $state<number | null>(null)
  let settingsOpen = $state(false)
  let brandRowEl: HTMLDivElement | undefined = $state()

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

  function selectUnified() {
    expandedAccountId = null
    onselect({ kind: 'unified', role: 'inbox' })
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
      </div>
    {/if}
  </div>

  <ul class="section">
    <li>
      <button
        class="item"
        class:active={isUnified}
        onclick={selectUnified}
        oncontextmenu={contextMenu({ kind: 'unified' })}
        onkeydown={contextKey({ kind: 'unified' })}
      >
        <Icon name="inbox" />
        <span class="label">Alle Posteingänge</span>
        {#if overview?.unread_inbox}<span class="count">{overview.unread_inbox}</span>{/if}
      </button>
    </li>
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
          class:active={view.kind === 'account' && view.accountId === account.id}
          class:parent-active={view.kind === 'folder' && view.accountId === account.id}
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
          {#if account.unread_inbox}<span class="count">{account.unread_inbox}</span>{/if}
        </button>

        {#if expanded && hasFolders}
          <ul
            class="folders"
            aria-label={`Ordner von ${account.display_name ?? accountLabel(account.email)}`}
            transition:slide={{ duration: EXPAND_MS, easing: cubicOut }}
          >
            {#each account.folders as folder, i (folder.id)}
              {@const folderActive = view.kind === 'folder' && view.folderId === folder.id}
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
                    {#if folder.unread}<span class="folder-count">{folder.unread}</span>{/if}
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

  <div class="cases-header">
    <button
      type="button"
      class="cases-title-btn"
      onclick={() => onshowcases?.()}
      title="Vorgänge-Übersicht anzeigen"
    >
      <h2>Vorgänge</h2>
    </button>
  </div>
  <ul class="section cases-scroll" aria-label="Vorgänge">
    {#each [...cases.filter((c) => c.status !== 'done' && c.status !== 'aside'), ...cases.filter((c) => c.status === 'done')] as c (c.id)}
      <li>
        <button
          type="button"
          class="item case-nav-item"
          class:active={selectedCaseId === c.id}
          class:muted-case={c.status === 'done'}
          onmouseenter={() => onhovercase?.(c.id)}
          onmouseleave={() => onhovercase?.(null)}
          onclick={() => onselectcase?.(c.id)}
          title={c.nextStep}
        >
          <span class="case-dot" class:urgent={c.status === 'needs_you'} class:done={c.status === 'done'}></span>
          <span class="label">{c.title}</span>
        </button>
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
  /* Folder counts stay quiet: a number, no pill. */
  .folder-count {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: var(--sidebar-text-muted);
    opacity: 0.85;
  }
  .folder-item.active .folder-count {
    color: var(--sidebar-text);
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
  .count {
    font-size: 12px;
    font-weight: 650;
    min-width: 22px;
    padding: 0 6px;
    border-radius: 99px;
    text-align: center;
    background: var(--sidebar-accent-fill);
    color: var(--sidebar-on-accent-fill);
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
  .cases-header {
    margin: 18px 8px 6px;
  }
  .cases-title-btn {
    border: 0;
    background: none;
    padding: 0;
    text-align: left;
  }
  .cases-title-btn h2 {
    margin: 0;
  }
  .cases-title-btn:hover h2 {
    color: var(--sidebar-accent-text);
  }
  /* Folders and cases share the height; both scroll on their own. */
  .accounts-scroll {
    flex: 0 1 auto;
    min-height: 80px;
    overflow-y: auto;
    scrollbar-width: thin;
  }
  .cases-scroll {
    flex: 1 1 0;
    min-height: 120px;
    overflow-y: auto;
    padding-bottom: 28px;
    mask-image: linear-gradient(to bottom, #000 72%, transparent 100%);
    -webkit-mask-image: linear-gradient(to bottom, #000 72%, transparent 100%);
    scrollbar-width: thin;
  }
  .case-nav-item {
    gap: 8px;
    padding: 5px 8px;
    font-size: 13px;
  }
  .case-nav-item.muted-case {
    color: var(--sidebar-text-muted);
    opacity: 0.65;
  }
  .case-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex: none;
    background: var(--sidebar-accent-fill);
    opacity: 0.7;
  }
  .case-dot.urgent {
    background: var(--ochre);
    opacity: 1;
  }
  .case-dot.done {
    background: var(--sidebar-text-muted);
    opacity: 0.5;
  }
</style>
