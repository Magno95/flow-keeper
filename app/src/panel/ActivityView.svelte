<script lang="ts">
  import { api } from "../lib/api";
  import Icon from "../lib/Icon.svelte";
  import { plural } from "../lib/format";
  import { failure } from "../lib/ui.svelte";
  import type { Activity, ActivityHour } from "../lib/types";
  import type { Route } from "./routes";
  import TokensView from "./TokensView.svelte";

  let { refresh, go }: { refresh: number; go: (r: Route) => void } = $props();

  type Metric = "panelOpens" | "conversationOpens";
  const metricLabel: Record<Metric, [string, string]> = {
    panelOpens: ["panel opening", "panel openings"],
    conversationOpens: ["conversation opened", "conversations opened"],
  };
  const PERIODS = [7, 30, 90];
  const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

  // Tokens of the conversations, or how you use the app.
  let tab = $state<"tokens" | "use">("tokens");
  let days = $state(30);
  let metric = $state<Metric>("panelOpens");
  let data = $state<Activity | null>(null);
  // What the pointer is on, shown above each chart.
  let hoverDay = $state<string | null>(null);
  let hoverHour = $state<string | null>(null);

  /** Local midnight `n` days before today (DST-safe). */
  function midnight(n: number): number {
    const d = new Date();
    d.setHours(0, 0, 0, 0);
    d.setDate(d.getDate() - n);
    return d.getTime();
  }
  const dayKey = (ts: number) => {
    const d = new Date(ts);
    return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
  };

  // The previous period too, for the comparison.
  const start = $derived(midnight(days - 1));
  const previousStart = $derived(midnight(2 * days - 1));

  $effect(() => {
    void refresh;
    const since = previousStart;
    api.activity(since).then((v) => (data = v), failure);
  });

  const current = $derived(data?.hours.filter((h) => h.hourStart >= start) ?? []);
  const previous = $derived(data?.hours.filter((h) => h.hourStart < start) ?? []);
  const sum = (hours: ActivityHour[], m: Metric) => hours.reduce((n, h) => n + h[m], 0);

  const daily = $derived.by(() => {
    const byDay = new Map<string, number>();
    for (const h of current) byDay.set(dayKey(h.hourStart), (byDay.get(dayKey(h.hourStart)) ?? 0) + h[metric]);
    const out: { ts: number; value: number }[] = [];
    for (let i = days - 1; i >= 0; i--) {
      const ts = midnight(i);
      out.push({ ts, value: byDay.get(dayKey(ts)) ?? 0 });
    }
    return out;
  });
  const dailyMax = $derived(Math.max(1, ...daily.map((d) => d.value)));
  const activeDays = $derived(new Set(current.filter((h) => h.panelOpens + h.conversationOpens > 0).map((h) => dayKey(h.hourStart))).size);

  // Rows Monday to Sunday, columns from midnight to 23.
  const heat = $derived.by(() => {
    const grid = Array.from({ length: 7 }, () => new Array<number>(24).fill(0));
    for (const h of current) {
      const d = new Date(h.hourStart);
      grid[(d.getDay() + 6) % 7][d.getHours()] += h[metric];
    }
    return grid;
  });
  const heatMax = $derived(Math.max(1, ...heat.flat()));
  const busiest = $derived.by(() => {
    let best: [number, number, number] = [0, 0, 0];
    heat.forEach((row, d) => row.forEach((v, h) => v > best[2] && (best = [d, h, v])));
    return best[2] ? `${WEEKDAYS[best[0]]} ${String(best[1]).padStart(2, "0")}:00` : null;
  });

  function delta(m: Metric): string | null {
    const before = sum(previous, m);
    // Without a full previous period the comparison means nothing.
    if (!before || !data?.since || data.since > previousStart) return null;
    const pct = Math.round(((sum(current, m) - before) / before) * 100);
    return `${pct > 0 ? "+" : pct < 0 ? "−" : "±"}${Math.abs(pct)}% vs previous ${days} days`;
  }

  const date = (ts: number, weekday = false) =>
    new Date(ts).toLocaleDateString("en-US", { day: "numeric", month: "short", ...(weekday ? { weekday: "short" } : {}) });
  const count = (n: number) => plural(n, ...metricLabel[metric]);
  const shade = (v: number) =>
    v ? `color-mix(in oklab, var(--chart) ${Math.round(18 + (82 * v) / heatMax)}%, var(--surface-muted))` : "var(--surface-muted)";
</script>

<header class="top">
  <button class="back" onclick={() => go({ name: "overview" })}><Icon name="back" size={16} /> Projects</button>
</header>
<div class="title">
  <h1>Analytics</h1>
  {#if tab === "tokens"}
    <p class="sub">Tokens of all your conversations, by project and model</p>
  {:else if data?.since}
    <p class="sub">How you use Flow Keeper, counted since {date(data.since)}</p>
  {/if}
</div>

<div class="scroll">
  <div class="tabs" role="tablist" aria-label="Analytics">
    {#each [["tokens", "Tokens"], ["use", "App use"]] as [k, label]}
      <button role="tab" aria-selected={tab === k} class:selected={tab === k} onclick={() => (tab = k as typeof tab)}>{label}</button>
    {/each}
  </div>

  {#if tab === "tokens"}
    <TokensView {refresh} {go} />
  {:else}
  <div class="periods" role="radiogroup" aria-label="Period">
    {#each PERIODS as n}
      <label class:selected={days === n}>
        <input type="radio" name="period" class="sr-only" checked={days === n} onchange={() => (days = n)} />
        {n} days
      </label>
    {/each}
  </div>

  {#if data && !data.since}
    <p class="empty">Nothing counted yet: open the panel and go to a conversation, and your use shows up here.</p>
  {:else if data}
    <div class="tiles" role="radiogroup" aria-label="Shown in the charts">
      {#each [["panelOpens", "Panel openings"], ["conversationOpens", "Conversations opened"]] as [m, label]}
        <label class="tile" class:selected={metric === m}>
          <input type="radio" name="metric" class="sr-only" checked={metric === m} onchange={() => (metric = m as Metric)} />
          <small>{label}</small>
          <strong>{sum(current, m as Metric).toLocaleString("en-US")}</strong>
          {#if delta(m as Metric)}<span>{delta(m as Metric)}</span>{/if}
        </label>
      {/each}
      <div class="tile">
        <small>Days used</small>
        <strong>{activeDays}<em>/{days}</em></strong>
      </div>
    </div>

    <section>
      <h2>Per day</h2>
      <p class="readout" aria-live="polite">{hoverDay ?? `Up to ${count(dailyMax)} a day`}</p>
      <div class="bars" class:dense={days > 30} role="list" onmouseleave={() => (hoverDay = null)}>
        {#each daily as d (d.ts)}
          {@const text = `${date(d.ts, true)} · ${count(d.value)}`}
          <div
            class="bar"
            role="listitem"
            aria-label={text}
            onmouseenter={() => (hoverDay = text)}
          >
            <span style:height={`${(d.value / dailyMax) * 100}%`}></span>
          </div>
        {/each}
      </div>
      <div class="axis"><span>{date(daily[0].ts)}</span><span>Today</span></div>
    </section>

    <section>
      <h2>When</h2>
      <p class="readout" aria-live="polite">
        {hoverHour ?? (busiest ? `Busiest hour: ${busiest}` : "No use in this period")}
      </p>
      <div class="heat" role="table" aria-label="Use by weekday and hour"
        onpointerover={(e) => (hoverHour = (e.target as HTMLElement).dataset.text ?? null)}
        onmouseleave={() => (hoverHour = null)}
      >
        {#each heat as row, d}
          <div class="heat-row" role="row">
            <span class="heat-day" role="rowheader">{WEEKDAYS[d]}</span>
            {#each row as v, h}
              {@const text = `${WEEKDAYS[d]} ${String(h).padStart(2, "0")}:00–${String(h + 1).padStart(2, "0")}:00 · ${count(v)}`}
              <span class="cell" role="cell" aria-label={text} data-text={text} style:background={shade(v)}></span>
            {/each}
          </div>
        {/each}
        <div class="heat-row hours" aria-hidden="true">
          <span class="heat-day"></span>
          {#each Array.from({ length: 24 }, (_, h) => h) as h}
            <span class="hour">{h % 6 === 0 ? h : ""}</span>
          {/each}
        </div>
      </div>
    </section>

    <p class="note">
      Only counts how many times the panel opens and takes you to a conversation, per hour: nothing about your
      conversations. It stays on this Mac unless a server is set in
      <button class="inline-link" onclick={() => go({ name: "settings" })}>Settings</button>.
    </p>
  {/if}
  {/if}
</div>

<style>
  .top {
    padding: 14px 14px 0 12px;
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 0;
    background: transparent;
    color: var(--text-2);
    font-weight: 600;
    font-size: 13.5px;
    padding: 6px 8px;
    border-radius: 999px;
  }
  .title {
    padding: 6px 20px 8px;
  }
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .sub {
    margin: 2px 0 0;
    color: var(--text-3);
    font-size: 13px;
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 4px 20px 24px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: var(--surface);
    border-radius: var(--radius-md);
    padding: 12px 14px;
    box-shadow: var(--shadow-card);
  }
  h2 {
    margin: 0;
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-3);
  }
  .tabs {
    display: flex;
    gap: 16px;
    border-bottom: 1px solid var(--line);
  }
  .tabs button {
    border: 0;
    background: none;
    padding: 4px 0 8px;
    margin-bottom: -1px;
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text-3);
    border-bottom: 2px solid transparent;
  }
  .tabs button.selected {
    color: var(--text);
    border-bottom-color: var(--chart);
  }
  .periods {
    display: flex;
    gap: 4px;
    padding: 3px;
    background: var(--surface-muted);
    border-radius: 999px;
    align-self: flex-start;
  }
  .periods label {
    padding: 5px 12px;
    border-radius: 999px;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text-2);
    cursor: pointer;
  }
  .periods label.selected {
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-card);
  }
  .periods label:has(:focus-visible),
  .tile:has(:focus-visible) {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  .tiles {
    display: grid;
    grid-template-columns: 1fr 1fr 0.8fr;
    gap: 8px;
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: var(--surface);
    border-radius: var(--radius-md);
    padding: 10px 12px;
    box-shadow: var(--shadow-card);
    border: 1.5px solid transparent;
    min-width: 0;
  }
  label.tile {
    cursor: pointer;
  }
  .tile.selected {
    border-color: var(--chart);
  }
  .tile small {
    color: var(--text-3);
    font-size: 12px;
  }
  .tile strong {
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
  }
  .tile em {
    font-style: normal;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-3);
  }
  .tile span {
    font-size: 11.5px;
    color: var(--text-2);
  }
  .readout {
    margin: 0;
    min-height: 18px;
    font-size: 12.5px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .bars {
    display: flex;
    align-items: stretch;
    gap: 2px;
    height: 110px;
    border-bottom: 1px solid var(--line);
  }
  .bars.dense {
    gap: 1px;
  }
  .bar {
    flex: 1;
    display: flex;
    align-items: flex-end;
    min-width: 0;
  }
  .bar span {
    display: block;
    width: 100%;
    background: var(--chart);
    border-radius: 4px 4px 0 0;
  }
  .bars.dense .bar span {
    border-radius: 2px 2px 0 0;
  }
  .bar:hover span {
    opacity: 0.75;
  }
  .axis {
    display: flex;
    justify-content: space-between;
    font-size: 11.5px;
    color: var(--text-3);
  }
  .heat {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .heat-row {
    display: grid;
    grid-template-columns: 30px repeat(24, 1fr);
    gap: 2px;
    align-items: center;
  }
  .heat-day,
  .hour {
    font-size: 11px;
    color: var(--text-3);
  }
  .cell {
    aspect-ratio: 1;
    border-radius: 3px;
  }
  .cell:hover {
    outline: 1.5px solid var(--text);
    outline-offset: -1px;
  }
  .empty,
  .note {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-3);
  }
  .empty {
    font-size: 13.5px;
    color: var(--text-2);
  }
  .inline-link {
    border: 0;
    background: none;
    padding: 0;
    font-size: inherit;
    color: var(--text-2);
    text-decoration: underline;
    text-underline-offset: 2px;
  }
</style>
