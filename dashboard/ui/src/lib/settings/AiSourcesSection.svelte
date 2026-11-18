<script lang="ts">
  // Filled in with the AI usage collector (next commits).
  import type { Meta } from "../types";

  interface Props {
    meta: Meta;
    onchanged: () => Promise<void>;
  }
  let { meta: _meta, onchanged: _onchanged }: Props = $props();
</script>

<h3>AI sources</h3>
<p class="muted">Not available yet.</p>
