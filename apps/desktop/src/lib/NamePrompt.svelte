<script lang="ts">
  // Asks for a bundle name next to the thing that asked; a small dialog, not
  // a page. Enter confirms, Escape cancels.
  import { onMount } from 'svelte'

  let {
    title,
    initial = '',
    confirm,
    anchor,
    error = null,
    onsubmit,
    oncancel,
  }: {
    title: string
    initial?: string
    confirm: string
    /** Where it hangs; focus returns there. */
    anchor: HTMLElement | null
    error?: string | null
    onsubmit: (name: string) => void
    oncancel: () => void
  } = $props()

  // svelte-ignore state_referenced_locally
  let name = $state(initial)
  let box: HTMLDivElement | undefined = $state()
  let input: HTMLInputElement | undefined = $state()
  let left = $state(0)
  let top = $state(0)
  const titleId = `np-${Math.random().toString(36).slice(2, 8)}`

  function portal(node: HTMLElement) {
    document.body.appendChild(node)
    return { destroy: () => node.remove() }
  }

  function cancel() {
    oncancel()
    if (anchor?.isConnected) anchor.focus()
  }

  onMount(() => {
    if (box && anchor) {
      const a = anchor.getBoundingClientRect()
      const r = box.getBoundingClientRect()
      left = Math.max(8, Math.min(a.left, window.innerWidth - r.width - 8))
      top = a.bottom + 8 + r.height > window.innerHeight - 8 ? Math.max(8, a.top - r.height - 8) : a.bottom + 8
    } else {
      left = Math.max(8, window.innerWidth / 2 - 150)
      top = 120
    }
    input?.focus()
    input?.select()
    const outside = (e: MouseEvent) => {
      if (box && !box.contains(e.target as Node)) cancel()
    }
    const t = setTimeout(() => window.addEventListener('mousedown', outside))
    return () => {
      clearTimeout(t)
      window.removeEventListener('mousedown', outside)
    }
  })
</script>

<div
  bind:this={box}
  use:portal
  class="prompt"
  role="dialog"
  aria-modal="true"
  aria-labelledby={titleId}
  tabindex="-1"
  style:left={`${left}px`}
  style:top={`${top}px`}
  onkeydown={(e) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      cancel()
    }
  }}
>
  <form
    onsubmit={(e) => {
      e.preventDefault()
      if (name.trim()) onsubmit(name.trim())
    }}
  >
    <p id={titleId} class="title">{title}</p>
    <input
      bind:this={input}
      bind:value={name}
      type="text"
      maxlength="60"
      placeholder="z. B. Verein"
      aria-label="Name des Bündels"
      autocomplete="off"
    />
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="row">
      <button type="button" class="btn quiet" onclick={cancel}>Abbrechen</button>
      <button type="submit" class="btn primary" disabled={!name.trim()}>{confirm}</button>
    </div>
  </form>
</div>

<style>
  .prompt {
    position: fixed;
    z-index: 90;
    width: 300px;
    max-width: calc(100vw - 16px);
    padding: 14px;
    border-radius: 16px;
    background: var(--surface-raised);
    box-shadow:
      0 14px 40px color-mix(in srgb, var(--pitch-black) 28%, transparent),
      0 0 0 1px color-mix(in srgb, var(--text) 9%, transparent);
  }
  .title {
    margin: 0 0 10px;
    font-size: 14px;
    font-weight: 650;
  }
  input {
    width: 100%;
    padding: 9px 12px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--bg);
    font-size: 14px;
  }
  input:focus-visible {
    outline: 2px solid var(--highlight);
    outline-offset: 1px;
  }
  .error {
    margin: 8px 0 0;
    font-size: 13px;
    color: var(--danger);
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 12px;
  }
  .btn {
    padding: 7px 14px;
    border: 0;
    border-radius: 99px;
    background: color-mix(in srgb, var(--text) var(--pill-mix), transparent);
    font-size: 14px;
    font-weight: 600;
  }
  .btn.quiet {
    background: none;
    color: var(--text-soft);
    font-weight: 500;
  }
  .btn.quiet:hover {
    background: color-mix(in srgb, var(--text) var(--pill-mix), transparent);
    color: var(--text);
  }
  .btn.primary {
    background: var(--accent-fill);
    color: var(--on-accent-fill);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
