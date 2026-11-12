<script lang="ts">
  import Icon from './Icon.svelte'
  import { initials, longDate } from './format'
  import type { MessageSummary } from './types'

  let { message }: { message: MessageSummary | null } = $props()

  const isNewsletter = $derived(!!message && message.header.list_unsubscribe.length > 0)
</script>

<article class="pane">
  {#if message}
    {@const h = message.header}
    <h1>{h.subject ?? '(kein Betreff)'}</h1>

    <div class="sender">
      <div class="avatar" aria-hidden="true">{initials(h.from_name, h.from_addr)}</div>
      <div class="who">
        <div class="name">{h.from_name ?? h.from_addr}</div>
        {#if h.from_name}<div class="addr">{h.from_addr}</div>{/if}
      </div>
      <time>{longDate(h.date)}</time>
    </div>

    <dl class="details">
      {#if h.to.length}<dt>An</dt><dd>{h.to.join(', ')}</dd>{/if}
      {#if h.cc.length}<dt>Kopie</dt><dd>{h.cc.join(', ')}</dd>{/if}
      <dt>Konto</dt><dd>{message.account_email} · {message.folder_path}</dd>
    </dl>

    {#if isNewsletter}
      <div class="hint">
        <Icon name="unsubscribe" />
        <span>
          Newsletter{h.list_unsubscribe_post ? ' – Abmelden mit einem Klick möglich' : ''}.
          Das Abbestellen kommt in einem der nächsten Schritte.
        </span>
      </div>
    {/if}

    <div class="body-placeholder">
      Der Mailtext wird noch nicht geladen – bisher synchronisiert Halcyon nur die Kopfzeilen.
    </div>
  {:else}
    <div class="nothing">
      <img src="/halcyon.svg" alt="" width="72" height="72" />
      <p>Wähle eine Nachricht aus.</p>
      <p class="keys"><kbd>↑</kbd> <kbd>↓</kbd> blättern · <kbd>/</kbd> suchen</p>
    </div>
  {/if}
</article>

<style>
  .pane {
    overflow-y: auto;
    padding: 24px 32px;
    background: var(--surface-raised);
    min-width: 0;
  }
  h1 {
    margin: 0 0 18px;
    font-size: 22px;
    font-weight: 650;
    line-height: 1.3;
  }
  .sender {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .avatar {
    width: 40px;
    height: 40px;
    flex: none;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-weight: 650;
    background: var(--accent-fill);
    color: var(--on-accent-fill);
  }
  .who {
    flex: 1;
    min-width: 0;
  }
  .name {
    font-weight: 600;
  }
  .addr,
  time {
    color: var(--text-muted);
    font-size: 13px;
  }
  .details {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 14px;
    margin: 16px 0;
    font-size: 13px;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .hint {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 10px 12px;
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--tangerine-dream) 22%, transparent);
    border: 1px solid color-mix(in srgb, var(--tangerine-dream) 55%, transparent);
    font-size: 13px;
    margin-bottom: 16px;
  }
  .body-placeholder {
    padding: 28px;
    border: 1px dashed var(--border);
    border-radius: var(--radius);
    color: var(--text-muted);
    text-align: center;
  }
  .nothing {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    color: var(--text-muted);
  }
  .nothing img {
    opacity: 0.9;
    margin-bottom: 8px;
  }
  .keys {
    font-size: 12px;
  }
  kbd {
    font-family: inherit;
    font-size: 11px;
    padding: 1px 5px;
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: 4px;
  }
</style>
