<script lang="ts">
  import { settings } from '../../stores/settings.svelte';
  import SettingToggle from '../SettingToggle.svelte';

  const value = $derived(settings.value);
</script>

{#if value}
  <h2>Connectors</h2>
  <p class="muted small">
    Watch your game launchers and post an action line like <code>* you plays Half-Life</code> into the room you're looking at when the game you're playing changes.
    Everything is off until you turn it on, because this shares what you're doing with other people.
  </p>
  <div class="divided">
    <SettingToggle title="Steam" hint="Detects the running Steam game." checked={value.connectors.steam_enabled} onchange={(on) => settings.update((d) => (d.connectors.steam_enabled = on))} />
    <SettingToggle title="GOG Galaxy" hint="Matches running processes against your installed GOG games." checked={value.connectors.gog_enabled} onchange={(on) => settings.update((d) => (d.connectors.gog_enabled = on))} />
    <SettingToggle title="Epic Games" hint="Matches running processes against your installed Epic games." checked={value.connectors.epic_enabled} onchange={(on) => settings.update((d) => (d.connectors.epic_enabled = on))} />
    <SettingToggle
      title="Announce when you stop"
      hint="Also post “stopped playing …” when you quit a game. Off by default; the start line is the interesting one."
      checked={value.connectors.announce_stop}
      onchange={(on) => settings.update((d) => (d.connectors.announce_stop = on))}
    />
    <div class="setting">
      <div class="what">
        <label for="poll">Check every</label>
        <div class="hint">Seconds between checks (at least 5). Looking at running processes is the costly part, so a longer interval is gentler.</div>
      </div>
      <input
        id="poll"
        class="input fit"
        type="number"
        min="5"
        max="600"
        value={value.connectors.poll_interval_secs}
        onchange={(e) => settings.update((d) => (d.connectors.poll_interval_secs = Math.max(5, Math.round(Number(e.currentTarget.value)) || 30)))}
      />
    </div>
  </div>
{/if}

<style>
  .fit {
    width: 90px;
  }
</style>
