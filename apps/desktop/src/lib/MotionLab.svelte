<script lang="ts">
  // Development-only test bench: finds an animation technique that ends
  // cleanly in WebKitGTK. There, transitions of transform/opacity showed the
  // start state again for one frame when they finished (design/interaktion.md).
  // All variants run in lockstep in an endless loop, so a short screen
  // recording of this page shows which ones flicker.
  import { onDestroy, onMount, tick } from 'svelte'
  import { fly } from 'svelte/transition'

  let { onclose }: { onclose: () => void } = $props()

  const IN_MS = 280
  const OUT_MS = 220
  const SHOW_FOR_MS = 1000
  const PERIOD_MS = 2000
  const SHIFT_PX = 28

  const variants = [
    { id: 'A', title: 'CSS-Übergang', note: 'opacity + transform, Klasse wechselt' },
    { id: 'B', title: 'CSS-Übergang + will-change', note: 'Ebene bleibt dauerhaft bestehen' },
    { id: 'C', title: 'CSS-Animation, forwards', note: '@keyframes, Endzustand von der Animation gehalten' },
    { id: 'D', title: 'Web Animations, commitStyles', note: 'fill: forwards, am Ende übernehmen und abbrechen' },
    { id: 'E', title: 'Web Animations, Ende vorab', note: 'Endzustand vorher gesetzt, Animation ohne fill' },
    { id: 'F', title: 'Svelte fly', note: 'Element wird ein- und ausgehängt' },
    { id: 'G', title: 'Ohne Grafikkarten-Ebene', note: 'top + Hintergrundfarbe mit Alpha' },
    { id: 'H', title: 'Feste 3D-Ebene', note: 'translate3d, backface hidden, isoliert' },
  ] as const

  // Round 2: the same plain transition everywhere, but inside the
  // surroundings where "Heute" flickered, each with or without one suspect.
  const contexts = [
    { id: 'A', title: 'Nur Deckkraft', note: 'Vergleich ohne besondere Umgebung' },
    { id: 'B', title: 'Deckkraft + Maske', note: 'Eltern mit mask-image wie die Bestell-Leiste' },
    { id: 'C', title: 'Deckkraft + Scrollen', note: 'waagerecht scrollende Leiste, ohne Maske' },
    { id: 'D', title: 'Deckkraft + Scrollen + Maske', note: 'genau wie die Bestell-Leiste' },
    { id: 'E', title: 'Verschieben + Scrollen + Maske', note: 'Kachel hebt sich in der Leiste' },
    { id: 'F', title: 'Verschieben + farbiger Schatten', note: 'Schatten mit color-mix, nur transform bewegt' },
    { id: 'G', title: 'Wie die alte Karte', note: 'Verlauf, Schatten und transform bewegen sich' },
    { id: 'H', title: 'Nach Einblend-Animation', note: 'animation both, danach transform-Übergang' },
  ] as const

  let round = $state<1 | 2>(2)
  let shown = $state(false)
  let cycles = $state(0)
  let boxD: HTMLDivElement | undefined = $state()
  let boxE: HTMLDivElement | undefined = $state()
  let timers: ReturnType<typeof setTimeout>[] = []
  let interval: ReturnType<typeof setInterval> | undefined

  const hiddenFrame = { opacity: 0, transform: `translateY(${SHIFT_PX}px)` }
  const shownFrame = { opacity: 1, transform: 'translateY(0)' }

  function runD(show: boolean) {
    if (!boxD) return
    const el = boxD
    const anim = el.animate(show ? [hiddenFrame, shownFrame] : [shownFrame, hiddenFrame], {
      duration: show ? IN_MS : OUT_MS,
      easing: show ? 'ease-out' : 'ease-in',
      fill: 'forwards',
    })
    anim.onfinish = () => {
      anim.commitStyles()
      anim.cancel()
    }
  }

  function runE(show: boolean) {
    if (!boxE) return
    const to = show ? shownFrame : hiddenFrame
    const from = show ? hiddenFrame : shownFrame
    boxE.style.opacity = String(to.opacity)
    boxE.style.transform = to.transform
    boxE.animate([from, to], {
      duration: show ? IN_MS : OUT_MS,
      easing: show ? 'ease-out' : 'ease-in',
    })
  }

  async function step(show: boolean) {
    shown = show
    if (show) cycles += 1
    await tick()
    runD(show)
    runE(show)
  }

  function cycle() {
    void step(true)
    timers.push(setTimeout(() => void step(false), SHOW_FOR_MS))
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault()
      onclose()
    }
  }

  onMount(() => {
    if (boxD) Object.assign(boxD.style, { opacity: '0', transform: hiddenFrame.transform })
    if (boxE) Object.assign(boxE.style, { opacity: '0', transform: hiddenFrame.transform })
    // Let the page settle before the first cycle, so it is not mistaken for a flicker.
    timers.push(
      setTimeout(() => {
        cycle()
        interval = setInterval(cycle, PERIOD_MS)
      }, 600),
    )
  })

  onDestroy(() => {
    timers.forEach(clearTimeout)
    if (interval) clearInterval(interval)
  })
</script>

<svelte:window onkeydown={onKey} />

<div class="lab" role="dialog" aria-modal="true" aria-label="Bewegungstest">
  <header>
    <div>
      <h1>Bewegungstest</h1>
      <p>
        Alle acht Felder blenden im Takt von zwei Sekunden ein und aus. Nimm etwa zehn Sekunden
        dieses Fensters auf und schick die Aufnahme. Halte die Maus dabei still.
      </p>
    </div>
    <div class="meta">
      <div class="rounds" role="group" aria-label="Testrunde">
        <button type="button" class:active={round === 1} onclick={() => (round = 1)}>Runde 1</button>
        <button type="button" class:active={round === 2} onclick={() => (round = 2)}>Runde 2</button>
      </div>
      <span class="count" aria-live="off">Durchlauf {cycles}</span>
      <button type="button" onclick={onclose}>Schließen</button>
    </div>
  </header>

  {#if round === 2}
    <ol class="grid">
      {#each contexts as v (v.id)}
        <li class="tile">
          <div class="stage r2 r2-{v.id.toLowerCase()}" class:on={shown}>
            {#if v.id === 'C' || v.id === 'D' || v.id === 'E'}
              <div class="scroller">
                <div class="track">
                  <span class="spacer"></span>
                  <div class="box"></div>
                  <span class="spacer"></span>
                  <span class="spacer"></span>
                </div>
              </div>
            {:else if v.id === 'B'}
              <div class="masked"><div class="box"></div></div>
            {:else}
              <div class="box"></div>
            {/if}
          </div>
          <p class="label"><strong>{v.id}</strong> {v.title}</p>
          <p class="note">{v.note}</p>
        </li>
      {/each}
    </ol>
  {:else}
  <ol class="grid">
    {#each variants as v (v.id)}
      <li class="tile">
        <div class="stage {v.id.toLowerCase()}" class:on={shown} class:off={!shown && cycles > 0}>
          {#if v.id === 'D'}
            <div class="box" bind:this={boxD}></div>
          {:else if v.id === 'E'}
            <div class="box" bind:this={boxE}></div>
          {:else if v.id === 'F'}
            {#if shown}
              <div
                class="box"
                in:fly={{ y: SHIFT_PX, duration: IN_MS }}
                out:fly={{ y: SHIFT_PX, duration: OUT_MS }}
              ></div>
            {/if}
          {:else}
            <div class="box"></div>
          {/if}
        </div>
        <p class="label"><strong>{v.id}</strong> {v.title}</p>
        <p class="note">{v.note}</p>
      </li>
    {/each}
  </ol>
  {/if}
</div>

<style>
  /* Fixed black and white on purpose: maximum contrast makes a single
     wrong frame measurable in a recording, independent of the theme. */
  .lab {
    position: fixed;
    inset: 0;
    z-index: 1000;
    overflow: auto;
    padding: 24px 28px;
    background: #000;
    color: #fff;
    font-size: 14px;
  }
  header {
    display: flex;
    gap: 24px;
    align-items: flex-start;
    justify-content: space-between;
    margin-bottom: 20px;
  }
  h1 {
    margin: 0 0 6px;
    font-size: 22px;
  }
  header p {
    margin: 0;
    max-width: 640px;
    color: #bbb;
    line-height: 1.5;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 14px;
    flex: none;
  }
  .count {
    font-variant-numeric: tabular-nums;
    color: #888;
  }
  button {
    border: 1px solid #555;
    background: #111;
    color: #fff;
    border-radius: 99px;
    padding: 6px 14px;
    cursor: pointer;
  }
  .grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(4, 220px);
    gap: 20px;
  }
  .tile {
    margin: 0;
  }
  .stage {
    position: relative;
    height: 150px;
    border: 1px solid #333;
    border-radius: 6px;
    overflow: hidden;
  }
  .box {
    position: absolute;
    left: 50px;
    top: 35px;
    width: 120px;
    height: 80px;
    border-radius: 4px;
    background: #fff;
  }
  .label {
    margin: 8px 0 2px;
  }
  .note {
    margin: 0;
    color: #888;
    font-size: 12px;
    line-height: 1.4;
  }

  .rounds {
    display: flex;
    gap: 4px;
  }
  .rounds button.active {
    background: #fff;
    color: #000;
  }

  /* ----- Round 2 ----- */
  .r2-a .box,
  .r2-b .box,
  .r2-c .box,
  .r2-d .box {
    opacity: 0.25;
    transition: opacity 220ms ease-in;
  }
  .r2-a.on .box,
  .r2-b.on .box,
  .r2-c.on .box,
  .r2-d.on .box {
    opacity: 1;
    transition: opacity 280ms ease-out;
  }
  .masked {
    position: absolute;
    inset: 0;
    -webkit-mask-image: linear-gradient(90deg, transparent, #000 28px, #000 calc(100% - 28px), transparent);
    mask-image: linear-gradient(90deg, transparent, #000 28px, #000 calc(100% - 28px), transparent);
  }
  .scroller {
    position: absolute;
    inset: 0;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }
  .scroller::-webkit-scrollbar {
    display: none;
  }
  .r2-d .scroller,
  .r2-e .scroller {
    -webkit-mask-image: linear-gradient(90deg, transparent, #000 28px, #000 calc(100% - 28px), transparent);
    mask-image: linear-gradient(90deg, transparent, #000 28px, #000 calc(100% - 28px), transparent);
  }
  .track {
    display: flex;
    align-items: center;
    height: 100%;
    width: max-content;
  }
  .track .box {
    position: relative;
    left: auto;
    top: auto;
    flex: none;
  }
  .spacer {
    flex: none;
    width: 50px;
    height: 10px;
  }
  .r2-e .box,
  .r2-f .box,
  .r2-g .box,
  .r2-h .box {
    transform: translateY(14px);
    transition:
      transform 220ms ease-in,
      box-shadow 220ms ease-in;
  }
  .r2-e.on .box,
  .r2-f.on .box,
  .r2-g.on .box,
  .r2-h.on .box {
    transform: translateY(-6px) scale(1.008);
    transition-duration: 280ms;
    transition-timing-function: ease-out;
  }
  .r2-f .box,
  .r2-g .box {
    --glow: #3fbf6f;
    box-shadow:
      0 1px 2px rgb(0 0 0 / 40%),
      0 14px 34px -16px color-mix(in srgb, var(--glow) 60%, transparent);
  }
  .r2-g .box {
    background: linear-gradient(160deg, color-mix(in srgb, var(--glow) 40%, #fff), #fff 72%);
  }
  .r2-g.on .box {
    box-shadow:
      0 2px 6px rgb(0 0 0 / 50%),
      0 18px 40px -14px color-mix(in srgb, var(--glow) 90%, transparent);
  }
  .r2-h .box {
    animation: lab-rise 420ms cubic-bezier(0.2, 0.7, 0.2, 1) both;
  }
  @keyframes lab-rise {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }

  /* A: plain transition */
  .a .box,
  .b .box {
    opacity: 0;
    transform: translateY(28px);
    transition:
      opacity 280ms ease-out,
      transform 280ms ease-out;
  }
  .a.on .box,
  .b.on .box {
    opacity: 1;
    transform: none;
  }
  .a.off .box,
  .b.off .box {
    transition-duration: 220ms;
    transition-timing-function: ease-in;
  }
  /* B: the layer is never dropped */
  .b .box {
    will-change: transform, opacity;
  }

  /* C: keyframes hold the end state */
  .c .box {
    opacity: 0;
    transform: translateY(28px);
  }
  .c.on .box {
    animation: lab-in 280ms ease-out forwards;
  }
  .c.off .box {
    animation: lab-out 220ms ease-in forwards;
  }
  @keyframes lab-in {
    from {
      opacity: 0;
      transform: translateY(28px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
  @keyframes lab-out {
    from {
      opacity: 1;
      transform: translateY(0);
    }
    to {
      opacity: 0;
      transform: translateY(28px);
    }
  }

  /* G: only main-thread properties (position and colour) */
  .g .box {
    top: 63px;
    background: rgb(255 255 255 / 0);
    transition:
      top 220ms ease-in,
      background-color 220ms ease-in;
  }
  .g.on .box {
    top: 35px;
    background: rgb(255 255 255 / 1);
    transition:
      top 280ms ease-out,
      background-color 280ms ease-out;
  }

  /* H: a permanent 3D layer on an isolated parent */
  .h {
    isolation: isolate;
    transform: translateZ(0);
  }
  .h .box {
    opacity: 0;
    transform: translate3d(0, 28px, 0);
    backface-visibility: hidden;
    transition:
      opacity 220ms ease-in,
      transform 220ms ease-in;
  }
  .h.on .box {
    opacity: 1;
    transform: translate3d(0, 0, 0);
    transition-duration: 280ms;
    transition-timing-function: ease-out;
  }
</style>
