<script lang="ts">
  import { errorText, sendCtl } from "./api";
  import Card from "./Card.svelte";
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

<div class="drive-health">
  <Card title="Drive Health" size="S">
    {#if snapshot}
      <p class="muted">Recorded {new Date(snapshot.time_ms).toLocaleString()}</p>
      <dl>
        <dt>Lifetime host reads</dt>
        <dd>{snapshot.host_bytes_read == null ? "Unavailable" : fmtBytes(snapshot.host_bytes_read)}</dd>
        <dt>Lifetime host writes</dt>
        <dd>{snapshot.host_bytes_written == null ? "Unavailable" : fmtBytes(snapshot.host_bytes_written)}</dd>
        <dt>Endurance used</dt><dd>{format(snapshot.endurance_used_percent, "%")}</dd>
        <dt>SSD temperature</dt><dd>{format(snapshot.temperature_c, " °C")}</dd>
        <dt>Power-on hours</dt><dd>{format(snapshot.power_on_hours, " h")}</dd>
      </dl>
    {/if}
    {#if status}<p class="muted">{status}</p>{/if}
    <div class="scan">
      <button disabled={busy || paused} onclick={scan}>{busy ? "Scanning…" : "Scan now"}</button>
      {#if message}<p role="status" class="muted">{message}</p>{/if}
    </div>
  </Card>
</div>

<style>
  .drive-health {
    display: grid;
    grid-template-columns: repeat(12, minmax(0, 1fr));
  }
  p, dl, dd {
    margin: 0;
  }
  dl {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 4px 12px;
  }
  dd {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .scan p {
    margin-top: 8px;
  }
</style>
