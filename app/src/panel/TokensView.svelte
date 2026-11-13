<script lang="ts">
  import { api } from "../lib/api";
  import { modelName, plural, tokens, usd } from "../lib/format";
  import { failure } from "../lib/ui.svelte";
  import type { Tokens, TokenShare, TokenStats } from "../lib/types";
  import type { Route } from "./routes";

  let { refresh, go }: { refresh: number; go: (r: Route) => void } = $props();

  type Metric = "cost" | "total" | "output" | "input" | "cacheWrite" | "cacheRead";
  /** The usage of a period, an hour, a project or a model. */
  type Amount = { tokens: Tokens; cost: number };
  const value: Record<Metric, (a: Amount) => number> = {
    cost: (a) => a.cost,
    total: ({ tokens: t }) => t.input + t.cacheWrite + t.cacheRead + t.output,
    output: (a) => a.tokens.output,
    input: (a) => a.tokens.input,
    cacheWrite: (a) => a.tokens.cacheWrite,
    cacheRead: (a) => a.tokens.cacheRead,
  };
  /** Tiles in order: the cost on its own row; the conversations tile closes the first row of tokens, the input side
   * fills the second. */
  const LABELS: [Metric, string][] = [
    ["cost", "Estimated cost"],
    ["total", "All tokens"],
    ["output", "Output tokens"],
    ["input", "Input tokens"],
    ["cacheWrite", "Cache write"],
    ["cacheRead", "Cache read"],
  ];
  /** Days; 1 = today, null = everything saved. */
  const PERIODS: (number | null)[] = [1, 7, 30, 90, null];
  const DAY_MS = 86_400_000;

  let days = $state<number | null>(30);
  let metric = $state<Metric>("cost");
  let data = $state<TokenStats | null>(null);
  let hoverBar = $state<string | null>(null);

  /** Local midnight `n` days before today (DST-safe). */
  function midnight(n: number): number {
    const d = new Date();
    d.setHours(0, 0, 0, 0);
    d.setDate(d.getDate() - n);
    return d.getTime();
  }

  const start = $derived(days === null ? null : midnight(days - 1));
  const previousStart = $derived(days === null ? null : midnight(2 * days - 1));

  /** Today is compared with yesterday up to the same hour, not the whole of it. */
  function sameHourYesterday(): number {
    const d = new Date();
    d.setDate(d.getDate() - 1);
    d.setHours(d.getHours() + 1, 0, 0, 0);
    return d.getTime();
  }

  $effect(() => {
    void refresh;
    api.tokenStats(start, previousStart, days === 1 ? sameHourYesterday() : null).then((v) => (data = v), failure);
  });

  const m = $derived(value[metric]);
  const unit = $derived(
    { cost: "", total: "tokens", output: "output tokens", input: "input tokens", cacheWrite: "cache write tokens", cacheRead: "cache read tokens" }[metric],
  );
  /** A figure of the selected metric: “$12.34”, “1.2M”. */
  const figure = (n: number) => (metric === "cost" ? usd(n) : tokens(n));
  const previous = $derived(data?.previous ? { tokens: data.previous, cost: data.previousCost ?? 0 } : null);

  function delta(current: Amount, before: Amount | null): string | null {
    const was = before ? m(before) : 0;
    // Without a full previous period the comparison means nothing.
    if (!was || !data?.firstHour || previousStart === null || data.firstHour > previousStart) return null;
    const pct = Math.round(((m(current) - was) / was) * 100);
    const vs = days === 1 ? "yesterday at this time" : `previous ${days} days`;
    return `${pct > 0 ? "+" : pct < 0 ? "−" : "±"}${Math.abs(pct)}% vs ${vs}`;
  }

  // Per hour today, per day, or per month when everything saved spans more than four months.
  type Step = "hour" | "day" | "month";
  const chart = $derived.by(() => {
    if (!data) return { step: "day" as Step, bars: [] };
    const from = start ?? midnightOf(data.firstHour ?? Date.now());
    const step: Step = days === 1 ? "hour" : start === null && Date.now() - from > 120 * DAY_MS ? "month" : "day";
    const key = (ts: number) => {
      const d = new Date(ts);
      const day = `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
      return step === "month" ? `${d.getFullYear()}-${d.getMonth()}` : step === "hour" ? `${day}-${d.getHours()}` : day;
    };
    const sums = new Map<string, number>();
    for (const h of data.hours) sums.set(key(h.hourStart), (sums.get(key(h.hourStart)) ?? 0) + m(h));
    const out: { ts: number; label: string; value: number }[] = [];
    const d = new Date(from);
    if (step === "month") d.setDate(1);
    while (d.getTime() <= Date.now()) {
      const ts = d.getTime();
      const label =
        step === "month"
          ? d.toLocaleDateString("en-US", { month: "short", year: "numeric" })
          : step === "hour"
            ? d.toLocaleTimeString("en-US", { hour: "numeric" })
            : d.toLocaleDateString("en-US", { weekday: "short", day: "numeric", month: "short" });
      out.push({ ts, label, value: sums.get(key(ts)) ?? 0 });
      if (step === "month") d.setMonth(d.getMonth() + 1);
      else if (step === "hour") d.setHours(d.getHours() + 1);
      else d.setDate(d.getDate() + 1);
    }
    return { step, bars: out };
  });
  const bars = $derived(chart.bars);
  const step = $derived(chart.step);
  const barMax = $derived(Math.max(0, ...bars.map((b) => b.value)) || 1);

  function midnightOf(ts: number): number {
    const d = new Date(ts);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  }

  const sorted = (list: TokenShare[]) => [...list].sort((a, b) => m(b) - m(a)).filter((s) => m(s) > 0);
  const projects = $derived(data ? sorted(data.projects) : []);
  const models = $derived(data ? sorted(data.models) : []);
  const total = $derived(data ? m(data) : 0);
  const share = (s: TokenShare) => (total ? Math.round((m(s) / total) * 100) : 0);
  const exact = (n: number) => n.toLocaleString("en-US");
  const exactUsd = (n: number) => n.toLocaleString("en-US", { style: "currency", currency: "USD" });
  const date = (ts: number) => new Date(ts).toLocaleDateString("en-US", { day: "numeric", month: "short", year: "numeric" });
  const breakdown = (t: Tokens) =>
    `${tokens(t.input)} input · ${tokens(t.cacheWrite)} cache write · ${tokens(t.cacheRead)} cache read · ${tokens(t.output)} output`;
</script>

<div class="periods" role="radiogroup" aria-label="Period">
  {#each PERIODS as n}
    <label class:selected={days === n}>
      <input type="radio" name="token-period" class="sr-only" checked={days === n} onchange={() => (days = n)} />
      {n === null ? "All" : n === 1 ? "Today" : `${n} days`}
    </label>
  {/each}
</div>

{#if data && !data.firstHour}
  <p class="empty">No tokens saved yet: they show up a minute after your conversations' first replies.</p>
{:else if data}
  <div class="tiles" role="radiogroup" aria-label="Shown in the charts">
    {#each LABELS as [k, label]}
      {@const v = value[k](data)}
      <label class="tile" class:wide={k === "cost"} class:selected={metric === k} title={k === "cost" ? exactUsd(v) : exact(v)}>
        <input type="radio" name="token-metric" class="sr-only" checked={metric === k} onchange={() => (metric = k)} />
        <small>{label}</small>
        <strong>{k === "cost" ? usd(v) : tokens(v)}</strong>
        {#if metric === k && delta(data, previous)}<span>{delta(data, previous)}</span>{/if}
        {#if k === "cost"}<span>Claude models at the Claude API list prices</span>{/if}
      </label>
      {#if k === "output"}
        <div class="tile">
          <small>Conversations</small>
          <strong>{exact(data.conversations)}</strong>
          <span>{plural(data.requests, "response", "responses")}</span>
        </div>
      {/if}
    {/each}
  </div>

  <section>
    <h2>{{ hour: "Per hour", day: "Per day", month: "Per month" }[step]}</h2>
    <p class="readout" aria-live="polite">{hoverBar ?? `Up to ${figure(barMax)}${unit && ` ${unit}`} ${{ hour: "an hour", day: "a day", month: "a month" }[step]}`}</p>
    <div class="bars" class:dense={bars.length > 31} role="list" onmouseleave={() => (hoverBar = null)}>
      {#each bars as b (b.ts)}
        {@const text = `${b.label} · ${figure(b.value)}${unit && ` ${unit}`}`}
        <div class="bar" role="listitem" aria-label={text} onmouseenter={() => (hoverBar = text)}>
          <span style:height={`${(b.value / barMax) * 100}%`}></span>
        </div>
      {/each}
    </div>
    {#if bars.length}
      <div class="axis"><span>{bars[0].label}</span><span>{{ hour: "Now", day: "Today", month: "This month" }[step]}</span></div>
    {/if}
  </section>

  {#each [["By project", projects], ["By model", models]] as [title, list]}
    {@const rows = list as TokenShare[]}
    {@const isProject = title === "By project"}
    <section>
      <h2>{title}</h2>
      {#if !rows.length}
        <p class="empty">Nothing in this period.</p>
      {/if}
      <ul class="shares">
        {#each rows as s (s.name + s.id)}
          <li title={isProject ? breakdown(s.tokens) : undefined}>
            <div class="share-head">
              {#if isProject}
                <button class="name link" onclick={() => go({ name: "project", id: s.id })}>{s.name}</button>
              {:else}
                <span class="name" title={s.name}>{modelName(s.name)}</span>
              {/if}
              <span class="figure">{figure(m(s))} <em>{share(s)}%</em></span>
            </div>
            <div class="track"><span style:width={`${(m(s) / m(rows[0])) * 100}%`}></span></div>
            {#if !isProject}<small class="types">{breakdown(s.tokens)}</small>{/if}
            <small>{plural(s.conversations, "conversation", "conversations")} · {plural(s.requests, "response", "responses")}</small>
          </li>
        {/each}
      </ul>
    </section>
  {/each}

  <p class="note">
    Saved by Flow Keeper on this Mac since {date(data.firstHour!)}, a minute after each reply: the totals stay even after
    Claude Code and Codex delete old transcripts. Conversations from before Flow Keeper aren't counted. Input tokens are
    the ones not read from the cache, and cache reads make up most of all tokens.
  </p>
  <p class="note">
    The estimated cost applies the Claude API list prices to the Claude models' tokens, with each cache write priced by
    how long it's kept. Codex and other agents aren't counted, nor fast mode, US-only inference, web searches or the
    discounts of a plan.
    {#if data.unpriced.length}No price yet for {data.unpriced.map(modelName).join(", ")}: not in the cost.{/if}
  </p>
{/if}

<style>
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
    grid-template-columns: repeat(3, 1fr);
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
  .tile.wide {
    grid-column: 1 / -1;
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
  .shares {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .shares li {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .share-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .name {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  button.name {
    border: 0;
    background: none;
    padding: 0;
    text-align: left;
  }
  button.name:hover {
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .figure {
    font-size: 13px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .figure em {
    font-style: normal;
    color: var(--text-3);
    font-weight: 500;
    margin-left: 4px;
  }
  .track {
    height: 6px;
    border-radius: 999px;
    background: var(--surface-muted);
    overflow: hidden;
  }
  .track span {
    display: block;
    height: 100%;
    background: var(--chart);
    border-radius: 999px;
  }
  .shares small {
    font-size: 11.5px;
    color: var(--text-3);
  }
  .shares small.types {
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
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
</style>
