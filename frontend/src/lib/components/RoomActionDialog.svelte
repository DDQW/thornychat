<script lang="ts">
  import { forgetRoom, joinRoom, leaveRoom, renameRoom } from '../actions';
  import { rooms } from '../stores/rooms.svelte';
  import { ui } from '../stores/ui.svelte';
  import Modal from './Modal.svelte';

  const action = $derived(ui.roomAction);
  const room = $derived(action && action.kind !== 'join' ? rooms.byId.get(action.roomId) : undefined);

  let input = $state('');
  let busy = $state(false);
  let error = $state('');

  // Seed the field when a dialog opens.
  $effect(() => {
    input = action?.kind === 'rename' ? (room?.name ?? '') : '';
    error = '';
  });

  function close() {
    ui.roomAction = null;
  }

  async function confirm(event?: SubmitEvent) {
    event?.preventDefault();
    if (!action || busy) return;
    busy = true;
    error = '';
    try {
      switch (action.kind) {
        case 'leave':
          await leaveRoom(action.roomId);
          break;
        case 'forget':
          await forgetRoom(action.roomId);
          break;
        case 'rename':
          if (!input.trim()) throw new Error('A room name can’t be empty.');
          await renameRoom(action.roomId, input.trim());
          break;
        case 'join':
          if (!input.trim()) throw new Error('Enter a room address like #room:server.');
          await joinRoom(input.trim());
          ui.toast(`Joined ${input.trim()}`, 'success', 3000);
          break;
      }
      close();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  const title = $derived(
    !action ? '' : action.kind === 'leave' ? 'Leave room' : action.kind === 'forget' ? 'Leave and forget room' : action.kind === 'rename' ? 'Rename room' : 'Join a room',
  );
  const verb = $derived(!action ? '' : action.kind === 'leave' ? 'Leave' : action.kind === 'forget' ? 'Leave and forget' : action.kind === 'rename' ? 'Rename' : 'Join');
</script>

{#if action}
  <Modal {title} onclose={close} width="440px">
    <form onsubmit={confirm} id="room-action">
      {#if action.kind === 'leave'}
        <p>Leave <strong>{room?.name ?? 'this room'}</strong>? You can rejoin later if it's public or you're invited again.</p>
      {:else if action.kind === 'forget'}
        <p>Leave <strong>{room?.name ?? 'this room'}</strong> and remove it from this device? Its local history is deleted, and it won't reappear.</p>
      {:else}
        <div class="field">
          <label for="room-input">{action.kind === 'rename' ? 'Room name' : 'Room address or id'}</label>
          <!-- svelte-ignore a11y_autofocus -->
          <input id="room-input" class="input" bind:value={input} autofocus spellcheck={action.kind === 'rename'} placeholder={action.kind === 'join' ? '#room:server or !roomid:server' : ''} />
        </div>
      {/if}
      {#if error}<p class="error-text" role="alert">{error}</p>{/if}
    </form>
    {#snippet footer()}
      <button class="btn" onclick={close}>Cancel</button>
      <button class="btn {action.kind === 'forget' ? 'danger' : 'primary'}" form="room-action" type="submit" disabled={busy}>{verb}</button>
    {/snippet}
  </Modal>
{/if}
