<script lang="ts">
  import { setDefaultNotificationMode } from '../../actions';
  import type { NotificationMode, NotificationScope } from '../../bindings';
  import { rooms } from '../../stores/rooms.svelte';
  import { ui } from '../../stores/ui.svelte';

  const MODES: { id: NotificationMode; label: string }[] = [
    { id: 'AllMessages', label: 'All messages' },
    { id: 'MentionsAndKeywordsOnly', label: 'Mentions & keywords only' },
    { id: 'Mute', label: 'Muted' },
  ];

  async function change(scope: NotificationScope, mode: NotificationMode) {
    try {
      await setDefaultNotificationMode(scope, mode);
    } catch (error) {
      ui.error(`Couldn't change the default: ${String(error)}`);
    }
  }
</script>

<h2>Default notifications</h2>
<p class="muted small">What a room follows when you haven't chosen anything for it. Changes sync with your other devices.</p>

<div class="setting">
  <div class="what"><label for="dm-default">Direct messages</label></div>
  <select id="dm-default" class="select fit" value={rooms.defaultModes.direct_messages} onchange={(e) => change('DirectMessages', e.currentTarget.value as NotificationMode)}>
    {#each MODES as mode (mode.id)}<option value={mode.id}>{mode.label}</option>{/each}
  </select>
</div>
<div class="setting">
  <div class="what"><label for="group-default">Group chats</label></div>
  <select id="group-default" class="select fit" value={rooms.defaultModes.group_chats} onchange={(e) => change('GroupChats', e.currentTarget.value as NotificationMode)}>
    {#each MODES as mode (mode.id)}<option value={mode.id}>{mode.label}</option>{/each}
  </select>
</div>

<h2>Per room</h2>
<p class="muted small">Right-click a room in the sidebar to mute it or change how it notifies you.</p>

<style>
  .fit {
    width: auto;
    min-width: 220px;
  }
</style>
