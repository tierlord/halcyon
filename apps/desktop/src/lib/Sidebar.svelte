<script lang="ts">
  import Icon from './Icon.svelte'
  import { accountLabel } from './format'
  import type { FolderInfo, FolderRole, Overview, View } from './types'

  let {
    overview,
    view,
    activeEmails = [],
    onselect,
    onmanageaccounts,
  }: {
    overview: Overview | null
    view: View
    /** Accounts being synced right now. */
    activeEmails?: string[]
    onselect: (v: View) => void
    onmanageaccounts: () => void
  } = $props()

  // Planned smart categories (see ROADMAP.md); shown so the direction is visible.
  const upcoming = ['Rechnungen', 'Bestellungen', 'Logindaten', 'Verträge', 'Newsletter']

  const isUnified = $derived(view.kind === 'unified')
  let expandedAccountId = $state<number | null>(null)

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

<nav class="sidebar" aria-label="Ordner">
  <div class="brand">
    <img src="/halcyon.svg" alt="" width="28" height="28" />
    <span>Halcyon</span>
  </div>

  <ul class="section">
    <li>
      <button
        class="item"
        class:active={isUnified}
        onclick={selectUnified}
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
  <ul class="section">
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
        >
          {#if hasFolders}
            <span class="chevron"><Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={14} /></span>
          {/if}
          <Icon name="mail" />
          <span class="label">{account.display_name ?? accountLabel(account.email)}</span>
          {#if account.unread_inbox}<span class="count">{account.unread_inbox}</span>{/if}
        </button>

        {#if expanded && hasFolders}
          <ul class="folders" aria-label={`Ordner von ${account.display_name ?? accountLabel(account.email)}`}>
            {#each account.folders as folder (folder.id)}
              {@const folderActive = view.kind === 'folder' && view.folderId === folder.id}
              <li>
                {#if folder.selectable}
                  <button
                    class="item folder-item"
                    class:active={folderActive}
                    style:padding-left={`${8 + Math.min(folder.depth, 4) * 12}px`}
                    title={folder.path}
                    onclick={() => selectFolder(account.id, folder)}
                  >
                    <Icon name={folderIcon(folder.role)} size={15} />
                    <span class="label">{folder.name}</span>
                    {#if folder.unread}<span class="count">{folder.unread}</span>{/if}
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

  <h2>Kategorien <span class="soon">bald</span></h2>
  <ul class="section upcoming">
    {#each upcoming as name}
      <li class="item disabled"><Icon name="tag" /><span class="label">{name}</span></li>
    {/each}
  </ul>
</nav>

<style>
  .sidebar {
    background: var(--surface);
    border-right: 1px solid var(--border);
    padding: 14px 10px;
    overflow-y: auto;
    user-select: none;
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
    color: var(--text-muted);
    transition: opacity 0.12s ease;
  }
  .section-header:hover .edit-btn,
  .section-header:focus-within .edit-btn {
    opacity: 1;
  }
  .edit-btn:hover {
    background: var(--hover);
    color: var(--text);
  }
  .add-link {
    display: block;
    margin-top: 4px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-text);
    font-weight: 600;
    font-size: 13px;
  }
  h2 {
    margin: 18px 8px 6px;
    font-size: 11px;
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .soon {
    text-transform: none;
    letter-spacing: 0;
    font-weight: 600;
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--ochre) 30%, transparent);
    color: var(--text);
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
    color: var(--text);
  }
  button.item:hover {
    background: var(--hover);
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
    background: var(--selection);
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
    color: var(--text-muted);
  }
  .chevron :global(svg) {
    color: var(--text-muted);
  }
  .folders {
    list-style: none;
    margin: 2px 0 6px 14px;
    padding: 0 0 0 6px;
    border-left: 1px solid var(--border);
  }
  .folder-item {
    gap: 8px;
    padding-top: 5px;
    padding-bottom: 5px;
    font-size: 13px;
  }
  .item :global(svg) {
    flex: none;
    color: var(--accent-text);
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
    background: var(--accent-fill);
    color: var(--on-accent-fill);
  }
  .disabled {
    color: var(--text-muted);
    cursor: default;
  }
  .disabled :global(svg) {
    color: var(--text-muted);
  }
  .empty {
    padding: 6px 8px;
    color: var(--text-muted);
    font-size: 13px;
  }
</style>
