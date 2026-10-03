<script lang="ts">
  import { joinCall, leaveCall } from '../actions';
  import { nameOf, avatarOf } from '../people';
  import { rooms } from '../stores/rooms.svelte';
  import { session } from '../stores/session.svelte';
  import { plural } from '../format';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';

  interface Props {
    roomId: string;
  }
  let { roomId }: Props = $props();

  const call = $derived(rooms.calls[roomId]);
  /** One face per person, even when someone joined from two devices. */
  const people = $derived([...new Set(call?.participants.map((p) => p.user_id) ?? [])]);
  let busy = $state(false);
  let error = $state('');

  async function toggle() {
    if (!call) return;
    busy = true;
    error = '';
    try {
      await (call.joined ? leaveCall(roomId) : joinCall(roomId));
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if call && (people.length > 0 || call.joined)}
  <div class="banner" role="status">
    <Icon name="phone" size={16} />
    <div class="faces">
      {#each people.slice(0, 6) as user (user)}
        <span title={nameOf(user, roomId)}><Avatar src={avatarOf(user, roomId)} name={nameOf(user, roomId)} seed={user} size={24} /></span>
      {/each}
    </div>
    <span class="grow">
      {plural(people.length, 'person', 'people')} in a call{call.joined ? ' — you have joined' : ''}
      <span class="muted small">· signaling only, no audio or video yet</span>
      {#if error}<span class="error-text small"> {error}</span>{/if}
    </span>
    <button class="btn small" class:primary={!call.joined} onclick={toggle} disabled={busy || !session.info}>
      {call.joined ? 'Leave' : 'Join'}
    </button>
  </div>
{/if}

<style>
  .banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--accent-wash);
    color: var(--accent);
  }
  .banner > .grow {
    color: var(--text);
  }
  .faces {
    display: flex;
  }
  .faces span {
    margin-right: -6px;
    border-radius: 50%;
    box-shadow: 0 0 0 2px var(--bg);
  }
</style>
