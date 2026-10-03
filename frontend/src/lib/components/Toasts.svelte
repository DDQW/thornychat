<script lang="ts">
  import { ui } from '../stores/ui.svelte';
  import Icon from './Icon.svelte';
</script>

<div class="toasts" aria-live="polite">
  {#each ui.toasts as toast (toast.id)}
    <div class="toast {toast.kind}" role="status">
      <span class="grow">{toast.text}</span>
      <button class="icon-btn" onclick={() => ui.dismissToast(toast.id)} aria-label="Dismiss"><Icon name="x" size={14} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 16px;
    bottom: 16px;
    z-index: 90;
    display: grid;
    gap: 8px;
    width: min(380px, calc(100vw - 32px));
    pointer-events: none;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--surface-strong);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.35);
    pointer-events: auto;
  }
  .toast.error {
    border-left-color: var(--danger);
  }
  .toast.success {
    border-left-color: var(--success);
  }
</style>
