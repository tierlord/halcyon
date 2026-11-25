<script lang="ts" module>
  export interface PopItem {
    label: string
    /** Shows a check mark when true; the entry is a menu item with a state. */
    checked?: boolean
    checkable?: boolean
    danger?: boolean
    disabled?: boolean
    /** A thin line above the entry. */
    separated?: boolean
    action: () => void
  }
</script>

<script lang="ts">
  // A small menu that hangs under a button. It is moved to <body> so that no
  // transformed or clipping ancestor (cards, the orders strip) can cut it off.
  import { onMount } from 'svelte'
  import Icon from './Icon.svelte'

  let {
    anchor,
    heading,
    items,
    label,
    onclose,
  }: {
    /** The button the menu belongs to; focus returns to it. */
    anchor: HTMLElement
    heading?: string
    items: PopItem[]
    /** Accessible name when there is no heading. */
    label?: string
    onclose: () => void
  } = $props()

  let menu: HTMLDivElement | undefined = $state()
  let left = $state(0)
  let top = $state(0)
  let placed = $state(false)

  function buttons(): HTMLButtonElement[] {
    return [...(menu?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])]
  }

  function close(refocus = true) {
    onclose()
    if (refocus && anchor.isConnected) anchor.focus()
  }

  function onKeydown(e: KeyboardEvent) {
    const list = buttons()
    const i = list.indexOf(document.activeElement as HTMLButtonElement)
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      close()
    } else if (e.key === 'ArrowDown') {
      e.preventDefault()
      list[(i + 1) % list.length]?.focus()
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      list[(i - 1 + list.length) % list.length]?.focus()
    } else if (e.key === 'Home') {
      e.preventDefault()
      list[0]?.focus()
    } else if (e.key === 'End') {
      e.preventDefault()
      list.at(-1)?.focus()
    } else if (e.key === 'Tab') {
      close(false)
    }
  }

  function run(item: PopItem) {
    close(false)
    item.action()
  }

  function portal(node: HTMLElement) {
    document.body.appendChild(node)
    return { destroy: () => node.remove() }
  }

  onMount(() => {
    if (menu) {
      const a = anchor.getBoundingClientRect()
      const r = menu.getBoundingClientRect()
      // Right-aligned under the button; flips above when the window ends.
      left = Math.max(8, Math.min(a.right - r.width, window.innerWidth - r.width - 8))
      top = a.bottom + 6 + r.height > window.innerHeight - 8 ? Math.max(8, a.top - r.height - 6) : a.bottom + 6
      placed = true
    }
    const first = menu?.querySelector<HTMLButtonElement>('button[aria-checked="true"]:not(:disabled)')
    ;(first ?? buttons()[0])?.focus()
    const outside = (e: MouseEvent) => {
      if (menu && !menu.contains(e.target as Node) && !anchor.contains(e.target as Node)) close(false)
    }
    // Next tick, so the click that opened the menu does not close it.
    const t = setTimeout(() => {
      window.addEventListener('mousedown', outside)
      window.addEventListener('blur', onclose)
      window.addEventListener('resize', onclose)
    })
    return () => {
      clearTimeout(t)
      window.removeEventListener('mousedown', outside)
      window.removeEventListener('blur', onclose)
      window.removeEventListener('resize', onclose)
    }
  })
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  bind:this={menu}
  use:portal
  class="pop"
  class:placed
  role="menu"
  tabindex="-1"
  aria-label={heading ?? label}
  style:left={`${left}px`}
  style:top={`${top}px`}
  onkeydown={onKeydown}
>
  {#if heading}<p class="head" aria-hidden="true">{heading}</p>{/if}
  {#each items as item}
    <button
      type="button"
      role={item.checkable ? 'menuitemcheckbox' : 'menuitem'}
      aria-checked={item.checkable ? !!item.checked : undefined}
      class:danger={item.danger}
      class:separated={item.separated}
      disabled={item.disabled}
      onclick={() => run(item)}
    >
      <span class="mark">{#if item.checked}<Icon name="check" size={14} />{/if}</span>
      <span>{item.label}</span>
    </button>
  {/each}
</div>

<style>
  .pop {
    position: fixed;
    z-index: 80;
    min-width: 220px;
    max-width: min(320px, calc(100vw - 16px));
    padding: 6px;
    border-radius: 14px;
    background: var(--surface-raised);
    box-shadow:
      0 12px 36px color-mix(in srgb, var(--pitch-black) 26%, transparent),
      0 0 0 1px color-mix(in srgb, var(--text) 9%, transparent);
    opacity: 0;
  }
  .pop.placed {
    opacity: 1;
    animation: pop 0.14s ease-out;
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-3px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pop.placed {
      animation: none;
    }
  }
  .head {
    margin: 4px 10px 6px;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--text-soft);
  }
  button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 0;
    border-radius: 9px;
    background: none;
    text-align: left;
    font-size: 14px;
    color: var(--text);
  }
  button.separated {
    margin-top: 4px;
    border-top-left-radius: 0;
    border-top-right-radius: 0;
    box-shadow: 0 -1px 0 color-mix(in srgb, var(--text) 10%, transparent);
  }
  button:hover:not(:disabled),
  button:focus-visible {
    background: var(--hover);
    outline: none;
  }
  button:disabled {
    color: var(--text-muted);
    cursor: default;
  }
  button.danger {
    color: var(--danger);
  }
  .mark {
    flex: none;
    width: 16px;
    display: inline-flex;
    color: var(--accent-text);
  }
</style>
