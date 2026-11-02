<script lang="ts">
  interface Props {
    title: string;
    hint?: string;
    checked: boolean;
    onchange: (checked: boolean) => void;
    disabled?: boolean;
  }
  let { title, hint, checked, onchange, disabled = false }: Props = $props();
  const id = `toggle-${Math.random().toString(36).slice(2, 9)}`;
</script>

<div class="setting">
  <div class="what">
    <label for={id} id="{id}-label">{title}</label>
    {#if hint}<div class="hint" id="{id}-hint">{hint}</div>{/if}
  </div>
  <button {id} class="switch" role="switch" aria-checked={checked} {disabled} onclick={() => onchange(!checked)} aria-labelledby="{id}-label" aria-describedby={hint ? `${id}-hint` : undefined}></button>
</div>

<style>
  label {
    cursor: pointer;
  }
  .switch:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
