<script lang="ts">
  import { settings } from '../../stores/settings.svelte';
  import SettingToggle from '../SettingToggle.svelte';

  const value = $derived(settings.value);
</script>

{#if value}
  <h2>Privacy</h2>
  <p class="muted small">A fresh install tells other people nothing about you until you opt in.</p>
  <div class="divided">
    <SettingToggle
      title="Send read receipts"
      hint="Let others see which message you've read, and when. When off, your read position still advances on your own devices (so unread badges clear) but is never shared."
      checked={value.privacy.send_read_receipts}
      onchange={(on) => settings.update((d) => (d.privacy.send_read_receipts = on))}
    />
    <SettingToggle
      title="Send typing notifications"
      hint="Show others a “typing…” indicator while you write a message."
      checked={value.privacy.send_typing_notifications}
      onchange={(on) => settings.update((d) => (d.privacy.send_typing_notifications = on))}
    />
    <SettingToggle
      title="Enable link previews"
      hint="Fetch previews for links in messages. When off, neither your homeserver nor third-party sites (Twitter, Steam, their image servers) are contacted to expand links you receive, which would otherwise reveal your IP address and what you're reading."
      checked={value.privacy.enable_link_previews}
      onchange={(on) => settings.update((d) => (d.privacy.enable_link_previews = on))}
    />
  </div>
{/if}
