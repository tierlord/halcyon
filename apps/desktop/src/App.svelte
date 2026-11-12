<script lang="ts">
  import { onMount } from 'svelte'
  import * as api from './lib/api'
  import Icon from './lib/Icon.svelte'
  import MessageList from './lib/MessageList.svelte'
  import ReadingPane from './lib/ReadingPane.svelte'
  import Sidebar from './lib/Sidebar.svelte'
  import { accountLabel } from './lib/format'
  import type { MessageSummary, Overview, SyncUpdate, View } from './lib/types'

  let overview: Overview | null = $state(null)
  let view: View = $state({ kind: 'unified', role: 'inbox' })
  let messages: MessageSummary[] = $state([])
  let loading = $state(true)
  let selectedId: number | null = $state(null)
  let query = $state('')
  let searchInput: HTMLInputElement | undefined = $state()
  let syncing = $state(false)
  let syncStatus = $state('')
  let errors: string[] = $state([])

  const selected = $derived(messages.find((m) => m.id === selectedId) ?? null)
  const searching = $derived(query.trim().length > 0)
  const title = $derived.by(() => {
    if (searching) return `Suche: ${query.trim()}`
    const v = view
    if (v.kind === 'unified') return 'Alle Posteingänge'
    const acc = overview?.accounts.find((a) => a.id === v.accountId)
    return acc ? `Posteingang · ${accountLabel(acc.email)}` : 'Posteingang'
  })

  // Requests can overlap while typing; only the newest result is shown.
  let requestSeq = 0
  async function load() {
    const seq = ++requestSeq
    loading = true
    try {
      const result = searching ? await api.search(query.trim()) : await api.messages(view)
      if (seq !== requestSeq) return
      messages = result
      if (!result.some((m) => m.id === selectedId)) selectedId = null
    } catch (e) {
      errors = [...errors, String(e)]
    } finally {
      if (seq === requestSeq) loading = false
    }
  }

  async function refreshOverview() {
    try {
      overview = await api.overview()
    } catch (e) {
      errors = [...errors, String(e)]
    }
  }

  let searchTimer: ReturnType<typeof setTimeout> | undefined
  function onSearchInput() {
    clearTimeout(searchTimer)
    searchTimer = setTimeout(load, 150)
  }

  function selectView(v: View) {
    view = v
    query = ''
    selectedId = null
    load()
  }

  function handleSync(u: SyncUpdate) {
    switch (u.kind) {
      case 'started':
        syncing = true
        syncStatus = 'Synchronisiere …'
        errors = []
        break
      case 'account':
        syncStatus = accountLabel(u.email)
        break
      case 'folder':
        syncStatus = `${accountLabel(u.email)} · ${u.path} ${u.done}/${u.total}`
        break
      case 'failed':
        errors = [...errors, `${u.email ? accountLabel(u.email) + ': ' : ''}${u.error}`]
        break
      case 'finished':
        syncing = false
        syncStatus =
          u.new_messages === 0
            ? 'Alles aktuell'
            : `${u.new_messages} neue Nachricht${u.new_messages === 1 ? '' : 'en'}`
        refreshOverview()
        load()
        break
    }
  }

  function moveSelection(delta: number) {
    if (messages.length === 0) return
    const index = messages.findIndex((m) => m.id === selectedId)
    const next = index < 0 ? 0 : Math.min(messages.length - 1, Math.max(0, index + delta))
    selectedId = messages[next].id
  }

  function onKeydown(e: KeyboardEvent) {
    const typing = e.target instanceof HTMLInputElement
    if (e.key === 'Escape' && typing) {
      query = ''
      searchInput?.blur()
      load()
      return
    }
    if (typing || e.ctrlKey || e.metaKey || e.altKey) {
      if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
        e.preventDefault()
        searchInput?.focus()
      }
      return
    }
    if (e.key === '/') {
      e.preventDefault()
      searchInput?.focus()
    } else if (e.key === 'ArrowDown' || e.key === 'j') {
      e.preventDefault()
      moveSelection(1)
    } else if (e.key === 'ArrowUp' || e.key === 'k') {
      e.preventDefault()
      moveSelection(-1)
    }
  }

  onMount(() => {
    refreshOverview()
    load()
    const unlisten = api.onSync(handleSync)
    return () => {
      unlisten.then((f) => f())
    }
  })
</script>

<svelte:window onkeydown={onKeydown} />

<div class="app">
  <Sidebar {overview} {view} onselect={selectView} />

  <main>
    <div class="toolbar">
      <label class="search">
        <Icon name="search" size={16} />
        <input
          bind:this={searchInput}
          bind:value={query}
          oninput={onSearchInput}
          type="search"
          placeholder="Alle Konten durchsuchen …"
          aria-label="Suche"
          spellcheck="false"
        />
        {#if !searching}<kbd>/</kbd>{/if}
      </label>

      <div class="status" aria-live="polite">
        {#if api.isMock}<span class="mock">Testdaten</span>{/if}
        <span>{syncStatus}</span>
      </div>

      <button
        class="sync"
        class:spinning={syncing}
        onclick={() => api.startSync()}
        disabled={syncing}
        title="Alle Konten abrufen"
      >
        <Icon name="sync" />
        <span>Abrufen</span>
      </button>
    </div>

    {#if errors.length}
      <div class="errors" role="alert">
        <ul>
          {#each errors as e}<li>{e}</li>{/each}
        </ul>
        <button class="close" onclick={() => (errors = [])} aria-label="Fehler ausblenden">
          <Icon name="x" size={16} />
        </button>
      </div>
    {/if}

    <div class="content">
      <MessageList
        {title}
        {messages}
        {selectedId}
        {loading}
        showAccount={searching || view.kind === 'unified'}
        onselect={(id) => (selectedId = id)}
      />
      <ReadingPane message={selected} />
    </div>
  </main>
</div>

<style>
  .app {
    display: grid;
    grid-template-columns: 240px 1fr;
    height: 100%;
  }
  main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border);
  }
  .search {
    flex: 0 1 460px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-muted);
  }
  .search:focus-within {
    border-color: var(--highlight);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--sky-aqua) 25%, transparent);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: none;
    color: var(--text);
  }
  kbd {
    font-family: inherit;
    font-size: 11px;
    padding: 0 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
  }
  .status {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 8px;
    align-items: center;
    justify-content: flex-end;
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mock {
    padding: 1px 8px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--tangerine-dream) 30%, transparent);
    color: var(--text);
    font-weight: 600;
  }
  .sync {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border: 0;
    border-radius: var(--radius);
    background: var(--accent-fill);
    color: var(--on-accent-fill);
    font-weight: 600;
  }
  .sync:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .sync:disabled {
    cursor: progress;
    opacity: 0.85;
  }
  .spinning :global(svg) {
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinning :global(svg) {
      animation: none;
    }
  }
  .errors {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    margin: 10px 16px 0;
    padding: 8px 12px;
    border-radius: var(--radius);
    border: 1px solid color-mix(in srgb, var(--danger) 50%, transparent);
    background: color-mix(in srgb, var(--danger) 10%, transparent);
    font-size: 13px;
  }
  .errors ul {
    flex: 1;
    margin: 0;
    padding-left: 16px;
  }
  .close {
    border: 0;
    background: none;
    color: var(--text-muted);
    padding: 2px;
  }
  .content {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(320px, 42%) 1fr;
  }
  .content > :global(*) {
    min-height: 0;
  }
</style>
