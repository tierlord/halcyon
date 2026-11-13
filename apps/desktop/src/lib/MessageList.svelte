<script lang="ts">
  import { onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import { getBrandLogo, subscribeLogos } from './brandLogos'
  import { accountLabel, listDate } from './format'
  import { isFlagged, isSeen, type MessageSummary } from './types'

  let {
    title,
    messages,
    selectedId,
    loading,
    showAccount,
    highlightedIds = [],
    chatMode = false,
    sentReplies = new Map<string, MessageSummary[]>(),
    onselect,
    onclosecase,
  }: {
    title: string
    messages: MessageSummary[]
    selectedId: number | null
    loading: boolean
    showAccount: boolean
    highlightedIds?: number[]
    chatMode?: boolean
    sentReplies?: Map<string, MessageSummary[]>
    onselect: (id: number) => void
    onclosecase?: () => void
  } = $props()

  let listEl: HTMLUListElement | undefined = $state()
  let logoVersion = $state(0)

  onMount(() => subscribeLogos(() => {
    logoVersion += 1
  }))

  // Keep the selected row visible during keyboard navigation.
  $effect(() => {
    if (selectedId == null || !listEl) return
    listEl.querySelector(`[data-id="${selectedId}"]`)?.scrollIntoView({ block: 'nearest' })
  })
</script>

<section class="list" aria-label={title}>
  <header>
    <div class="head-left">
      {#if chatMode && onclosecase}
        <button type="button" class="back-btn" onclick={onclosecase} title="Zurück zur normalen Liste">
          ←
        </button>
      {/if}
      <h1>{title}</h1>
    </div>
    <span class="meta">{loading ? 'Lädt …' : `${messages.length} Nachrichten`}</span>
  </header>

  {#if !loading && messages.length === 0}
    <p class="empty">Hier ist nichts. Herrlich ruhig.</p>
  {/if}

  <ul bind:this={listEl} class:chat-list={chatMode} role="listbox" aria-label={title}>
    {#each messages as m (m.id)}
      {@const unread = !isSeen(m)}
      {@const isOutgoing = m.folder_role === 'sent'}
      {@const logo = (void logoVersion, !isOutgoing && m.header.dmarc_pass ? getBrandLogo(m.header.from_addr) : null)}
      {@const replies = !chatMode && m.header.message_id ? (sentReplies.get(m.header.message_id) ?? []) : []}
      <li
        role="option"
        aria-selected={m.id === selectedId}
        data-id={m.id}
        class:selected={m.id === selectedId}
        class:unread
        class:case-highlighted={highlightedIds.includes(m.id)}
        class:chat-in={chatMode && !isOutgoing}
        class:chat-out={chatMode && isOutgoing}
      >
        <button class:has-brand={!!logo} onclick={() => onselect(m.id)}>
          {#if logo?.bg}
            <span class="brand-wash" style:--brand-wash={logo.bg} aria-hidden="true"></span>
          {/if}
          <span class="dot" aria-label={unread ? 'ungelesen' : undefined}></span>
          {#if logo}
            <span class="brand" aria-hidden="true">
              <img
                src={logo.dataUrl}
                alt=""
                class:opaque={!logo.padded}
                style:transform={logo.scale !== 1 ? `scale(${logo.scale})` : undefined}
              />
            </span>
          {/if}
          <span class="from">
            {#if chatMode && isOutgoing}Du · Gesendet{:else}{m.header.from_name ?? m.header.from_addr ?? 'Unbekannt'}{/if}
          </span>
          <time>
            {#if isFlagged(m)}<span class="flag" title="Markiert"><Icon name="star" size={12} filled /></span>{/if}
            {listDate(m.header.date)}
          </time>
          <span class="subject">
            {#if m.has_attachments}<Icon name="paperclip" size={13} />{/if}
            {m.header.subject ?? '(kein Betreff)'}{#if m.preview}<span class="preview"> – {m.preview}</span>{/if}
          </span>
          {#if showAccount && !chatMode}<span class="account">{accountLabel(m.account_email)}</span>{/if}
        </button>

        {#if replies.length > 0}
          <div class="inline-replies">
            {#each replies as rep (rep.id)}
              <button
                type="button"
                class="inline-reply"
                class:selected={rep.id === selectedId}
                onclick={() => onselect(rep.id)}
                title="Eigene Antwort öffnen"
              >
                <span class="reply-tag">Du · {listDate(rep.header.date)}</span>
                <span class="reply-text">{rep.preview ?? rep.header.subject ?? '(Antwort)'}</span>
              </button>
            {/each}
          </div>
        {/if}
      </li>
    {/each}
  </ul>
</section>

<style>
  .list {
    display: flex;
    flex-direction: column;
    min-width: 0;
    border-right: 1px solid var(--border);
    background: var(--bg);
  }
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    padding: 16px 18px 10px;
  }
  h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 650;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta {
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
  }
  .empty {
    padding: 40px 18px;
    text-align: center;
    color: var(--text-muted);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0 8px 12px;
    overflow-y: auto;
    flex: 1;
  }
  li button {
    position: relative;
    overflow: hidden;
    width: 100%;
    display: grid;
    grid-template-columns: 10px 1fr auto;
    grid-template-areas:
      'dot from date'
      '.   subject account';
    column-gap: 10px;
    row-gap: 2px;
    align-items: center;
    padding: 9px 10px;
    border: 0;
    border-radius: var(--radius);
    background: none;
    text-align: left;
  }
  li button.has-brand {
    grid-template-columns: 10px 30px 1fr auto;
    grid-template-areas:
      'dot brand from date'
      '.   brand subject account';
  }
  .brand-wash {
    position: absolute;
    inset: 0;
    pointer-events: none;
    z-index: 0;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--brand-wash) 92%, transparent) 0%,
      color-mix(in srgb, var(--brand-wash) 88%, transparent) 46px,
      color-mix(in srgb, var(--brand-wash) 34%, transparent) 88px,
      color-mix(in srgb, var(--brand-wash) 10%, transparent) 132px,
      transparent 180px
    );
  }
  li.selected button .brand-wash {
    left: 3px;
  }
  .brand {
    grid-area: brand;
    position: relative;
    z-index: 1;
    width: 30px;
    height: 30px;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    flex-shrink: 0;
  }
  .brand img {
    width: 24px;
    height: 24px;
    object-fit: contain;
    display: block;
  }
  .brand img.opaque {
    width: 28px;
    height: 28px;
    object-fit: cover;
    border-radius: 6px;
  }
  .dot,
  .from,
  time,
  .subject,
  .account {
    position: relative;
    z-index: 1;
  }
  li button:hover {
    background: var(--hover);
  }
  li.selected button {
    background: var(--selection);
    box-shadow: inset 3px 0 0 var(--highlight);
  }
  .dot {
    grid-area: dot;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .unread .dot {
    background: var(--unread);
  }
  .from {
    grid-area: from;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .unread .from,
  .unread .subject {
    font-weight: 650;
  }
  time {
    grid-area: date;
    justify-self: end;
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .flag {
    color: var(--ochre);
    margin-right: 4px;
    vertical-align: -1px;
  }
  .unread time {
    color: var(--accent-text);
    font-weight: 600;
  }
  .subject {
    grid-area: subject;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-muted);
  }
  .unread .subject {
    color: var(--text);
  }
  .subject :global(svg) {
    vertical-align: -2px;
    color: var(--text-muted);
  }
  .preview {
    font-weight: 400;
    color: var(--text-muted);
  }
  .account {
    grid-area: account;
    justify-self: end;
    font-size: 11px;
    padding: 0 6px;
    border-radius: 99px;
    border: 1px solid var(--border);
    color: var(--text-muted);
    white-space: nowrap;
  }
  .head-left {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .back-btn {
    border: 1px solid var(--border);
    background: var(--surface);
    border-radius: var(--radius);
    padding: 2px 8px;
    font-size: 13px;
    color: var(--accent-text);
  }
  .back-btn:hover {
    background: var(--hover);
  }
  li.case-highlighted > button {
    background: color-mix(in srgb, var(--highlight) 12%, transparent);
    box-shadow: inset 3px 0 0 var(--highlight);
  }
  ul.chat-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 6px;
  }
  li.chat-in > button {
    max-width: 92%;
    margin-right: auto;
    background: var(--surface-raised);
    border: 1px solid var(--border);
  }
  li.chat-out > button {
    max-width: 86%;
    margin-left: auto;
    background: var(--selection);
    border: 1px solid var(--border);
  }
  .inline-replies {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
    margin: 2px 6px 6px 34px;
  }
  .inline-reply {
    width: auto !important;
    max-width: 88%;
    display: flex !important;
    flex-direction: column;
    align-items: flex-end;
    gap: 1px;
    padding: 5px 10px !important;
    border-radius: var(--radius);
    background: var(--surface) !important;
    border: 1px solid var(--border) !important;
    text-align: right !important;
  }
  .inline-reply:hover {
    background: var(--hover) !important;
  }
  .inline-reply.selected {
    background: var(--selection) !important;
    border-color: var(--highlight) !important;
  }
  .reply-tag {
    font-size: 10px;
    font-weight: 650;
    color: var(--accent-text);
  }
  .reply-text {
    font-size: 12px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
</style>
