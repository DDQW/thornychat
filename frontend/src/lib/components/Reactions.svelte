<script lang="ts">
  import { toggleReaction } from '../actions';
  import { backend } from '../api';
  import type { ReactionGroup } from '../bindings';
  import { isUnicodeEmoji } from '../emoji';
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

  /**
   * What a reaction key draws as. Keys are opaque strings: Cinny (and this
   * client) key custom emoji on their `mxc://` URL, some clients on a
   * `:shortcode:` resolved against the room's packs, the rest are unicode
   * emoji or plain words.
   */
  function visual(key: string): { kind: 'image'; src: string; name: string } | { kind: 'unicode' } | { kind: 'text' } {
    if (key.startsWith('mxc://')) {
      const shortcode = emoji.byMxc.get(key)?.shortcode;
      return { kind: 'image', src: backend.mediaUrl(key), name: shortcode ? `:${shortcode}:` : 'custom emoji' };
    }
    const custom = /^:[\w+-]{1,64}:$/.test(key) ? emoji.resolve(key) : undefined;
    if (custom) return { kind: 'image', src: backend.mediaUrl(custom.mxc_url), name: `:${custom.shortcode}:` };
    return isUnicodeEmoji(key) ? { kind: 'unicode' } : { kind: 'text' };
  }
</script>

{#if reactions.length > 0}
  <div class="reactions">
    {#each reactions as group (group.key)}
      {@const look = visual(group.key)}
      <button class="pill" class:mine={group.reacted_by_me} title={look.kind === 'image' ? `${look.name}: ${who(group)}` : who(group)} onclick={() => toggle(group.key)} aria-pressed={group.reacted_by_me}>
        {#if look.kind === 'image'}
          <img src={look.src} alt={look.name} loading="lazy" draggable="false" />
        {:else if look.kind === 'unicode'}
          <Twemoji char={group.key} size={16} />
        {:else}
          <span class="text">{group.key}</span>
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
  /* Holds an 18 px square while loading; a wide emoji then keeps its shape. */
  .pill img {
    width: auto;
    min-width: 18px;
    max-width: 40px;
    height: 18px;
    object-fit: contain;
  }
  .text {
    max-width: 12em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    font-variant-numeric: tabular-nums;
  }
</style>
