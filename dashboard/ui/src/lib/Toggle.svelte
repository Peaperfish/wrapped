<script lang="ts">
  // An on / off switch (role="switch"). Space and Return toggle it.
  interface Props {
    checked: boolean;
    label: string;
    disabled?: boolean;
    onchange: (on: boolean) => void;
  }
  let { checked, label, disabled = false, onchange }: Props = $props();
</script>

<button
  type="button"
  class="switch"
  class:on={checked}
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  onclick={() => onchange(!checked)}
>
  <span class="knob"></span>
</button>

<style>
  .switch {
    position: relative;
    width: 36px;
    height: 20px;
    padding: 0;
    border-radius: 999px;
    border: 1px solid var(--card-border);
    background: var(--switch-off);
    flex: none;
    transition: background-color 0.15s ease;
  }
  .switch.on {
    background: rgba(var(--accent-rgb), 0.85);
    border-color: transparent;
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--switch-knob);
    transition: transform 0.15s ease;
  }
  .on .knob {
    transform: translateX(16px);
  }
</style>
