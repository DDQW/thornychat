<script lang="ts">
  import { ui } from '../stores/ui.svelte';
  import Icon from './Icon.svelte';
</script>

<!-- Errors interrupt (role=alert); everything else waits its turn (role=status).
     Pointing at or tabbing into the stack holds every toast on screen. -->
<div
  class="toasts"
  role="region"
  aria-label="Notifications"
  onmouseenter={() => ui.pauseToasts(true)}
  onmouseleave={() => ui.pauseToasts(false)}
  onfocusin={() => ui.pauseToasts(true)}
  onfocusout={(event) => !(event.currentTarget as Element).contains(event.relatedTarget as Node | null) && ui.pauseToasts(false)}
>
  {#each ui.toasts as toast (toast.id)}
    <div class="toast {toast.kind}" role={toast.kind === 'error' ? 'alert' : 'status'}>
      <span class="mark" aria-hidden="true"><Icon name={toast.kind === 'error' ? 'info' : toast.kind === 'success' ? 'check' : 'bell'} size={15} /></span>
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
    animation: rise 0.16s ease-out;
  }
  /* The kind is in an icon as well as the stripe: colour alone says nothing to some readers. */
  .mark {
    display: inline-flex;
    color: var(--accent);
  }
  .toast.error {
    border-left-color: var(--danger);
  }
  .toast.error .mark {
    color: var(--danger);
  }
  .toast.success {
    border-left-color: var(--success);
  }
  .toast.success .mark {
    color: var(--success);
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
</style>
