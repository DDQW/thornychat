<script lang="ts">
  import { toggleReaction } from '../actions';
  import { backend } from '../api';
  import type { ReactionGroup } from '../bindings';
  import { nameOf } from '../people';
  import { emoji } from '../stores/emoji.svelte';
  import { ui } from '../stores/ui.svelte';
  import Twemoji from './Twemoji.svelte';

  interface Props {
    roomId: string;
    eventId: string;
    reactions: ReactionGroup[];
  }
  let { roomId, eventId, reactions }: Props = $props();

  function toggle(key: string) {
    toggleReaction(roomId, eventId, key).catch((error: unknown) => ui.error(`Couldn't react: ${String(error)}`));
  }

  /** "alice, bob and 2 others" — who sent a reaction, for the hover title. */
  function who(group: ReactionGroup): string {
    const names = group.senders.map((sender) => nameOf(sender, roomId));
    if (names.length <= 3) return names.join(', ');
    return `${names.slice(0, 3).join(', ')} and ${names.length - 3} others`;
  }
</script>

{#if reactions.length > 0}
  <div class="reactions">
    {#each reactions as group (group.key)}
      <button class="pill" class:mine={group.reacted_by_me} title={who(group)} onclick={() => toggle(group.key)} aria-pressed={group.reacted_by_me}>
        {#if group.key.startsWith('mxc://')}
          <img src={backend.mediaUrl(group.key)} alt={emoji.byMxc.get(group.key)?.shortcode ?? 'custom emoji'} loading="lazy" />
        {:else if /^:[\w-]+:$/.test(group.key)}
          <span class="text">{group.key}</span>
        {:else}
          <Twemoji char={group.key} size={16} />
        {/if}
        <span class="count">{group.count}</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .reactions {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
  }
  /* No background of their own: a reaction is a tint, not a button. */
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 8px;
    border: 1px solid transparent;
    border-radius: 12px;
    background: var(--hover);
    font-size: 12px;
  }
  .pill:hover {
    border-color: var(--border);
  }
  .pill.mine {
    border-color: var(--accent);
    background: var(--accent-wash);
  }
  .pill img {
    width: 16px;
    height: 16px;
    object-fit: contain;
  }
  .count {
    font-variant-numeric: tabular-nums;
  }
</style>
