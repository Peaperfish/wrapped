<script lang="ts">
  import { errorText, sendCtl } from "./api";
  import { fmtBytes } from "./format";
  import type { DriveHealthMeta } from "./types";

  interface Props {
    health: DriveHealthMeta;
    reload: () => Promise<DriveHealthMeta | null>;
    paused: boolean;
  }
  let { health, reload, paused }: Props = $props();
  let busy = $state(false);
  let message = $state("");
  const snapshot = $derived(health.snapshot);
  const status = $derived.by(() => {
    switch (health.status) {
      case "helper_missing":
        return "Wrapped couldn’t find smartctl. Drive Health requires an existing smartmontools installation.";
      case "no_internal_nvme":
        return "Wrapped couldn’t find an internal NVMe SSD.";
      case "health_unavailable":
        return "Wrapped couldn’t read usable health data from the SSD.";
      case "timed_out":
        return "The SSD health scan timed out.";
      default:
        return snapshot ? "" : "No SSD health reading yet. Scan now or wait for the daily scan.";
    }
  });

  function format(value: number | null, unit: string) {
    return value == null ? "Unavailable" : `${value.toLocaleString()}${unit}`;
  }

  async function scan() {
    busy = true;
    message = "";
    const before = health.last_scan_ms;
    try {
      const result = await sendCtl("scan", { what: "drive_health" });
      const skip = result.skipped.drive_health;
      if (skip && skip !== "already running") {
        message = skip;
        return;
      }
      const deadline = Date.now() + 8000;
      while (Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 750));
        const current = await reload();
        if (!current || current.last_scan_ms !== before) return;
      }
      message = "Scan still pending. Refresh the dashboard to check again.";
    } catch (e) {
      message = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="drive-health">
  <header>
    <h3>Drive Health</h3>
    <button disabled={busy || paused} onclick={scan}>{busy ? "Scanning…" : "Scan now"}</button>
  </header>
  {#if snapshot}
    <p class="muted">Recorded {new Date(snapshot.time_ms).toLocaleString()}</p>
    <dl>
      <div>
        <dt>Lifetime host reads</dt>
        <dd>{snapshot.host_bytes_read == null ? "Unavailable" : fmtBytes(snapshot.host_bytes_read)}</dd>
      </div>
      <div>
        <dt>Lifetime host writes</dt>
        <dd>{snapshot.host_bytes_written == null ? "Unavailable" : fmtBytes(snapshot.host_bytes_written)}</dd>
      </div>
      <div><dt>Endurance used</dt><dd>{format(snapshot.endurance_used_percent, "%")}</dd></div>
      <div><dt>SSD temperature</dt><dd>{format(snapshot.temperature_c, " °C")}</dd></div>
      <div><dt>Power-on hours</dt><dd>{format(snapshot.power_on_hours, " h")}</dd></div>
    </dl>
  {/if}
  {#if status}<p class="muted">{status}</p>{/if}
  {#if message}<p role="status" class="muted">{message}</p>{/if}
</section>

<style>
  .drive-health {
    min-width: 0;
    padding-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  header,
  dl > div {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }
  h3, p, dl, dd {
    margin: 0;
  }
  h3 {
    font-size: 13px;
    font-weight: 600;
  }
  button {
    font-size: 12px;
    padding: 2px 8px;
    white-space: nowrap;
  }
  p, dl {
    font-size: 12px;
  }
  dl > div {
    padding-block: 4px;
  }
  dl > div + div {
    border-top: 1px solid var(--divider);
  }
  dt {
    color: var(--text-dim);
  }
  dd {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
