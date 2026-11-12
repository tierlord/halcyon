<script lang="ts">
  import Icon from './Icon.svelte'
  import { accountLabel } from './format'
  import type { Overview, View } from './types'

  let {
    overview,
    view,
    onselect,
    onmanageaccounts,
  }: {
    overview: Overview | null
    view: View
    onselect: (v: View) => void
    onmanageaccounts: () => void
  } = $props()

  // Planned smart categories (see ROADMAP.md); shown so the direction is visible.
  const upcoming = ['Rechnungen', 'Bestellungen', 'Logindaten', 'Verträge', 'Newsletter']

  const isUnified = $derived(view.kind === 'unified')
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
        onclick={() => onselect({ kind: 'unified', role: 'inbox' })}
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
      <li>
        <button
          class="item"
          class:active={view.kind === 'account' && view.accountId === account.id}
          title={account.email}
          onclick={() => onselect({ kind: 'account', accountId: account.id, role: 'inbox' })}
        >
          <Icon name="mail" />
          <span class="label">{account.display_name ?? accountLabel(account.email)}</span>
          {#if account.unread_inbox}<span class="count">{account.unread_inbox}</span>{/if}
        </button>
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
    background: color-mix(in srgb, var(--tangerine-dream) 30%, transparent);
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
  .item.active {
    background: var(--selection);
    box-shadow: inset 3px 0 0 var(--highlight);
    font-weight: 600;
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
