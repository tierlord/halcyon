<script lang="ts" module>
  export interface MenuItem {
    label: string
    icon?: 'mailopen' | 'trash' | 'sync' | 'pencil'
    danger?: boolean
    disabled?: boolean
    action: () => void
  }
</script>

<script lang="ts">
  import { onMount } from 'svelte'
  import Icon from './Icon.svelte'

  let {
    x,
    y,
    items,
    onclose,
  }: { x: number; y: number; items: MenuItem[]; onclose: () => void } = $props()

  let menu: HTMLDivElement | undefined = $state()
  // svelte-ignore state_referenced_locally
  let left = $state(x)
  // svelte-ignore state_referenced_locally
  let top = $state(y)

  function buttons(): HTMLButtonElement[] {
    return [...(menu?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])]
  }

  function onKeydown(e: KeyboardEvent) {
    const list = buttons()
    const i = list.indexOf(document.activeElement as HTMLButtonElement)
    if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      onclose()
    } else if (e.key === 'ArrowDown') {
      e.preventDefault()
      list[(i + 1) % list.length]?.focus()
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      list[(i - 1 + list.length) % list.length]?.focus()
    } else if (e.key === 'Tab') {
      onclose()
    }
  }

  function run(item: MenuItem) {
    onclose()
    item.action()
  }

  onMount(() => {
    // Keep the menu inside the window.
    if (menu) {
      const r = menu.getBoundingClientRect()
      left = Math.min(x, window.innerWidth - r.width - 8)
      top = Math.min(y, window.innerHeight - r.height - 8)
    }
    buttons()[0]?.focus()
    const outside = (e: MouseEvent) => {
      if (menu && !menu.contains(e.target as Node)) onclose()
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

<div
  bind:this={menu}
  class="menu"
  role="menu"
  tabindex="-1"
  style:left={`${left}px`}
  style:top={`${top}px`}
  onkeydown={onKeydown}
>
  {#each items as item}
    <button
      type="button"
      role="menuitem"
      class:danger={item.danger}
      disabled={item.disabled}
      onclick={() => run(item)}
    >
      {#if item.icon}<Icon name={item.icon} size={15} />{:else}<span class="no-icon"></span>{/if}
      <span>{item.label}</span>
    </button>
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 40;
    min-width: 220px;
    padding: 4px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--surface-raised);
    box-shadow: 0 10px 32px color-mix(in srgb, var(--pitch-black) 25%, transparent);
    animation: pop 0.12s ease-out;
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-3px) scale(0.98);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .menu {
      animation: none;
    }
  }
  button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    border: 0;
    border-radius: 6px;
    background: none;
    text-align: left;
    font-size: 13px;
  }
  button:hover:not(:disabled),
  button:focus-visible {
    background: var(--hover);
    outline: none;
  }
  button :global(svg) {
    color: var(--accent-text);
    flex: none;
  }
  button.danger {
    color: var(--danger);
  }
  button.danger :global(svg) {
    color: var(--danger);
  }
  button:disabled {
    color: var(--text-muted);
    cursor: default;
  }
  button:disabled :global(svg) {
    color: var(--text-muted);
  }
  .no-icon {
    width: 15px;
    flex: none;
  }
</style>
