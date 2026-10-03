<script lang="ts">
  import { backend } from '../api';
  import { ui } from '../stores/ui.svelte';
  import Icon from './Icon.svelte';

  interface Props {
    mxcUrl: string;
    filename: string;
  }
  let { mxcUrl, filename }: Props = $props();

  let saving = $state(false);

  async function save() {
    saving = true;
    try {
      if (await backend.saveMedia(mxcUrl, filename)) ui.toast(`Saved ${filename}`, 'success', 3000);
    } catch (error) {
      ui.error(String(error));
    } finally {
      saving = false;
    }
  }
</script>

<button class="file" onclick={save} disabled={saving} title="Save {filename}">
  <span class="icon"><Icon name="file" size={22} /></span>
  <span class="name truncate">{filename}</span>
  <span class="action">{#if saving}<span class="spinner"></span>{:else}<Icon name="download" size={16} />{/if}</span>
</button>

<style>
  .file {
    display: flex;
    align-items: center;
    gap: 10px;
    width: min(360px, 100%);
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    text-align: left;
  }
  .file:hover:not(:disabled) {
    background: var(--surface-strong);
  }
  .icon {
    color: var(--accent);
  }
  .name {
    flex: 1 1 auto;
    min-width: 0;
  }
  .action {
    color: var(--muted);
  }
</style>
