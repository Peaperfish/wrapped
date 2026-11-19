<script lang="ts">
  // The Now tab: live hardware cards in the same 12-column grid as the
  // Overview. A card whose values are all none is not shown (fanless Mac,
  // Windows for now); a single none value shows "—".
  import Card from "../Card.svelte";
  import Ring from "../Ring.svelte";
  import Meter from "./Meter.svelte";
  import Spark from "./Spark.svelte";
  import { DASH, celsius, gb, pct, rate } from "./format";
  import { nowSampler as s } from "./sampler.svelte";

  const snap = $derived(s.snap);
  const hist = $derived(s.hist);

  const anyOf = (...v: unknown[]) => v.some((x) => x != null);

  const cores = $derived(snap?.cpu_cores ?? []);
  const clusters = $derived(
    (
      [
        ["P", "Performance"],
        ["E", "Efficiency"],
        [null, "Cores"],
      ] as const
    )
      .map(([k, title]) => ({
        title,
        cores: cores.filter((c) => (c.kind === "P" || c.kind === "E" ? c.kind : null) === k),
      }))
      .filter((g) => g.cores.length > 0),
  );

  const showCpu = $derived(
    !!snap && anyOf(snap.cpu_percent, snap.cpu_p_percent, snap.cpu_e_percent, snap.cpu_cores),
  );
  const showGpu = $derived(!!snap && anyOf(snap.gpu_percent));
  const showMem = $derived(
    !!snap &&
      anyOf(snap.mem_used_bytes, snap.mem_total_bytes, snap.mem_pressure, snap.mem_compressed_bytes, snap.swap_used_bytes),
  );
  const showDisk = $derived(!!snap && anyOf(snap.disk_read_per_sec, snap.disk_write_per_sec));
  const showNet = $derived(!!snap && anyOf(snap.net_down_per_sec, snap.net_up_per_sec));

  const memFrac = $derived(
    snap?.mem_used_bytes != null && snap.mem_total_bytes ? snap.mem_used_bytes / snap.mem_total_bytes : null,
  );
  const pressure = $derived(snap?.mem_pressure);

  // Temperature rows: the P-core group disappears while that cluster sleeps.
  const P = "CPU performance cores";
  const E = "CPU efficiency cores";
  const ORDER = [P, E, "GPU"];
  const temps = $derived.by(() => {
    const t = snap?.temps;
    if (!t || t.length === 0) return [];
    const by = new Map(t.map((x) => [x.label, x.celsius]));
    const labels = [
      ...ORDER,
      ...[...by.keys()].filter((l) => !ORDER.includes(l)).sort(),
    ].filter((l) => l === P || by.has(l));
    return labels.map((label) => {
      const v = by.get(label);
      const text = v != null ? celsius(v) : label === P && by.has(E) ? "idle" : DASH;
      return { label, text };
    });
  });

  // Nothing at all after two samples (rates need two): not supported here.
  const allNone = $derived(
    !!snap &&
      !showCpu &&
      !showGpu &&
      !showMem &&
      !showDisk &&
      !showNet &&
      temps.length === 0 &&
      (snap.fans ?? []).length === 0,
  );
  const unavailable = $derived(allNone && hist.cpu.length >= 2);

  const fans = $derived(snap?.fans ?? []);
  const showFans = $derived(fans.length > 0);
</script>

{#if s.error && !snap}
  <section class="card"><p class="error">{s.error}</p></section>
{:else if unavailable}
  <section class="card"><p class="muted">Live hardware stats aren't available on this platform yet</p></section>
{:else if !snap || allNone}
  <section class="card"><p class="muted">Reading…</p></section>
{:else}
  <div class="grid">
    {#if showCpu}
      <Card title="CPU" subtitle={snap.chip ?? undefined} size="M" empty={false}>
        <div class="cpu-top">
          <Ring fraction={(snap.cpu_percent ?? 0) / 100} center={snap.cpu_percent == null ? DASH : `${pct(snap.cpu_percent)}%`} caption="total" />
          <dl class="split">
            <dt>Performance</dt>
            <dd class="num">{pct(snap.cpu_p_percent)}<span class="u">%</span></dd>
            <dt>Efficiency</dt>
            <dd class="num">{pct(snap.cpu_e_percent)}<span class="u">%</span></dd>
          </dl>
          <div class="spark"><Spark series={[hist.cpu]} max={100} label="CPU, last 60 seconds" /></div>
        </div>
        {#each clusters as g (g.title)}
          <div class="cluster">
            <p class="cl-title faint">{g.title}</p>
            <div class="cores">
              {#each g.cores as c, i (i)}
                <div class="core">
                  <span class="cn faint num">{g.title[0]}{i + 1}</span>
                  <Meter fraction={c.percent == null ? null : c.percent / 100} height={6} label="{g.title} core {i + 1}" />
                  <span class="cv num">{pct(c.percent)}</span>
                </div>
              {/each}
            </div>
          </div>
        {/each}
      </Card>
    {/if}

    {#if showMem}
      <Card title="Memory" size="M" empty={false}>
        <div class="row">
          <span class="big num">{gb(snap.mem_used_bytes)}</span>
          <span class="muted">of <span class="num">{gb(snap.mem_total_bytes)}</span></span>
          <span class="spacer"></span>
          <span
            class="pressure"
            class:warn={pressure === "warning"}
            class:crit={pressure === "critical"}>{pressure ?? DASH}</span
          >
        </div>
        <Meter fraction={memFrac} height={10} label="Memory used" />
        <dl class="kv">
          <dt>Compressed</dt>
          <dd class="num">{gb(snap.mem_compressed_bytes)}</dd>
          <dt>Swap used</dt>
          <dd class="num">{gb(snap.swap_used_bytes)}</dd>
          <dt>Swap in</dt>
          <dd class="num">{rate(snap.swap_in_per_sec).join(" ")}</dd>
          <dt>Swap out</dt>
          <dd class="num">{rate(snap.swap_out_per_sec).join(" ")}</dd>
        </dl>
      </Card>
    {/if}

    {#if showGpu}
      <Card title="GPU" size="S" empty={false}>
        <div class="gpu">
          <Ring fraction={(snap.gpu_percent ?? 0) / 100} center={snap.gpu_percent == null ? DASH : `${pct(snap.gpu_percent)}%`} caption="busy" />
        </div>
        <Spark series={[hist.gpu]} max={100} label="GPU, last 60 seconds" />
      </Card>
    {/if}

    {#if showDisk}
      {@const r = rate(snap.disk_read_per_sec)}
      {@const w = rate(snap.disk_write_per_sec)}
      <Card title="Disk" size="S" empty={false}>
        <dl class="kv">
          <dt>Read</dt>
          <dd><span class="v num">{r[0]}</span><span class="u">{r[1]}</span></dd>
          <dt>Write</dt>
          <dd><span class="v num">{w[0]}</span><span class="u">{w[1]}</span></dd>
        </dl>
        <Spark series={[hist.diskRead, hist.diskWrite]} label="Disk read and write, last 60 seconds" />
      </Card>
    {/if}

    {#if showNet}
      {@const d = rate(snap.net_down_per_sec)}
      {@const u = rate(snap.net_up_per_sec)}
      <Card title="Network" size="S" empty={false}>
        <dl class="kv">
          <dt>Down</dt>
          <dd><span class="v num">{d[0]}</span><span class="u">{d[1]}</span></dd>
          <dt>Up</dt>
          <dd><span class="v num">{u[0]}</span><span class="u">{u[1]}</span></dd>
        </dl>
        <Spark series={[hist.netDown, hist.netUp]} label="Network down and up, last 60 seconds" />
      </Card>
    {/if}

    {#if temps.length > 0}
      <Card title="Temperatures" size="S" empty={false}>
        <dl class="kv">
          {#each temps as t (t.label)}
            <dt>{t.label.replace("CPU ", "").replace(/^./, (c) => c.toUpperCase())}</dt>
            <dd class="num">{t.text}</dd>
          {/each}
        </dl>
      </Card>
    {/if}

    {#if showFans}
      <Card title="Fans" size="S" empty={false}>
        {#each fans as f (f.name)}
          <div class="fan">
            <div class="row">
              <span class="muted">{f.name}</span>
              <span class="spacer"></span>
              <span class="num">{Math.round(f.rpm)}<span class="u"> rpm</span></span>
            </div>
            <Meter fraction={f.max_rpm ? f.rpm / f.max_rpm : null} height={6} label="{f.name} speed" />
          </div>
        {/each}
      </Card>
    {/if}
  </div>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
    gap: 16px;
  }
  .cpu-top {
    display: grid;
    grid-template-columns: auto auto 1fr;
    align-items: center;
    gap: 20px;
  }
  .spark {
    min-width: 0;
  }
  .split {
    display: grid;
    grid-template-columns: auto auto;
    gap: 4px 14px;
    margin: 0;
  }
  dt {
    color: var(--text-dim);
  }
  dd {
    margin: 0;
    text-align: right;
  }
  .split dd {
    min-width: 4ch;
  }
  .kv {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 6px 12px;
    margin: 0;
  }
  .v {
    display: inline-block;
    min-width: 5ch;
    text-align: right;
  }
  .u {
    display: inline-block;
    min-width: 5ch;
    margin-left: 4px;
    text-align: left;
    color: var(--text-dim);
    font-size: 12px;
  }
  .split .u {
    min-width: 0;
    margin-left: 1px;
  }
  .row {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .spacer {
    flex: 1;
  }
  .big {
    font-size: 24px;
    font-weight: 600;
    min-width: 8ch;
  }
  .pressure {
    font-weight: 600;
    text-transform: capitalize;
    color: var(--accent);
  }
  .pressure.warn {
    color: var(--warn);
  }
  .pressure.crit {
    color: var(--danger);
  }
  .gpu {
    display: grid;
    place-items: center;
  }
  .cl-title {
    margin: 0 0 6px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .cores {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 6px 18px;
  }
  .core {
    display: grid;
    grid-template-columns: 3ch 1fr 3ch;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }
  .cv {
    text-align: right;
  }
  .fan {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
</style>
