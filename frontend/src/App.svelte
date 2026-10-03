<script lang="ts">
  import { onMount } from 'svelte';
  import { postActivityEmote } from './lib/actions';
  import { backend } from './lib/api';
  import ContextMenu from './lib/components/ContextMenu.svelte';
  import Login from './lib/components/Login.svelte';
  import Shell from './lib/components/Shell.svelte';
  import Toasts from './lib/components/Toasts.svelte';
  import { handleEvent } from './lib/events';
  import { emoji } from './lib/stores/emoji.svelte';
  import { previews } from './lib/stores/previews.svelte';
  import { rooms } from './lib/stores/rooms.svelte';
  import { security } from './lib/stores/security.svelte';
  import { session } from './lib/stores/session.svelte';
  import { timelines } from './lib/stores/timelines.svelte';
  import { ui } from './lib/stores/ui.svelte';

  // Sign-out leaves nothing of the previous account on screen.
  session.onSignedOut = () => {
    rooms.reset();
    timelines.reset();
    emoji.reset();
    previews.reset();
    security.reset();
    ui.reset();
  };
  session.onSignedIn = () => void emoji.load();

  onMount(() => {
    let stopEmotes = () => {};
    void (async () => {
      // The listener must exist before anything can produce an event.
      await backend.attachEvents(handleEvent);
      stopEmotes = await backend.onConnectorEmote(postActivityEmote);
      await session.boot();
    })();
    return () => stopEmotes();
  });

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && ui.closeTopmost()) event.preventDefault();
  }

  /**
   * Links in messages open in the default browser, never in this window: the
   * page has IPC access, so it must never navigate away. Only web and mail
   * links are passed on; the shell re-checks.
   */
  function openLinks(event: MouseEvent) {
    if (event.defaultPrevented || event.button > 1) return;
    const link = (event.target as Element | null)?.closest?.('a[href]');
    if (!link) return;
    event.preventDefault();
    const href = link.getAttribute('href') ?? '';
    if (/^(https?:|mailto:)/i.test(href)) {
      void backend.openExternal(href).catch((error: unknown) => ui.error(`Couldn't open the link: ${String(error)}`));
    }
  }

  /** The native menu stays for text fields (spelling suggestions, cut/copy/paste); elsewhere there is none. */
  function oncontextmenu(event: MouseEvent) {
    const target = event.target as Element | null;
    const editable = target?.closest?.('input, textarea, [contenteditable="true"]');
    const selection = window.getSelection()?.toString();
    if (!editable && !selection) event.preventDefault();
  }
</script>

<svelte:window {onkeydown} onclick={openLinks} onauxclick={openLinks} {oncontextmenu} />

{#if session.phase === 'booting'}
  <div class="boot" role="status" aria-label="Starting ThornyChat"><span class="spinner"></span></div>
{:else if session.phase === 'login'}
  <Login />
{:else}
  <Shell />
{/if}

<ContextMenu />
<Toasts />

<style>
  .boot {
    display: grid;
    height: 100%;
    place-items: center;
  }
</style>
