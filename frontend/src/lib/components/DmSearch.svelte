<script lang="ts">
  import { createRoomWith, openDirectMessage, searchUsers } from '../actions';
  import type { UserSearchResult } from '../bindings';
  import { friendlyUserId } from '../format';
  import { looksLikeUser } from '../slash';
  import { ui } from '../stores/ui.svelte';
  import Avatar from './Avatar.svelte';
  import Modal from './Modal.svelte';

  let query = $state('');
  let results = $state<UserSearchResult[]>([]);
  let limited = $state(false);
  let searching = $state(false);
  let error = $state('');
  let busyUser = $state<string | null>(null);

  // Search the homeserver's user directory as you type (debounced); a stale
  // answer to an earlier query is ignored.
  let latest = 0;
  $effect(() => {
    const text = query.trim();
    error = '';
    if (text.length < 2) {
      results = [];
      limited = false;
      searching = false;
      return;
    }
    const ticket = ++latest;
    searching = true;
    const timer = setTimeout(async () => {
      try {
        const answer = await searchUsers(text);
        if (ticket !== latest) return;
        results = answer.results;
        limited = answer.limited;
      } catch (e) {
        if (ticket === latest) error = String(e);
      } finally {
        if (ticket === latest) searching = false;
      }
    }, 250);
    return () => clearTimeout(timer);
  });

  // Someone not in the directory (no shared room) can still be messaged by id.
  const typedId = $derived(looksLikeUser(query.trim()) && !results.some((r) => r.user_id === query.trim()) ? query.trim() : null);

  async function run(userId: string, make: (id: string) => Promise<void>, what: string) {
    busyUser = userId;
    error = '';
    try {
      await make(userId);
      // The worker answers with an event that opens the room and closes this dialog.
    } catch (e) {
      error = `Couldn't ${what}: ${String(e)}`;
    } finally {
      busyUser = null;
    }
  }
</script>

<Modal title="Start a conversation" onclose={() => (ui.dmSearchOpen = false)} width="480px">
  <!-- svelte-ignore a11y_autofocus -->
  <input class="input" placeholder="Search by name or @user:server" bind:value={query} autofocus spellcheck="false" aria-label="Search people" />

  <ul class="results">
    {#if typedId}
      <li>
        <Avatar name={friendlyUserId(typedId)} seed={typedId} size={34} />
        <div class="grow"><div>{typedId}</div><div class="muted small">Message this user id</div></div>
        <button class="btn primary small" disabled={busyUser !== null} onclick={() => run(typedId, openDirectMessage, 'open the conversation')}>Message</button>
      </li>
    {/if}
    {#each results as user (user.user_id)}
      <li>
        <Avatar src={user.avatar_url} name={user.display_name || friendlyUserId(user.user_id)} seed={user.user_id} size={34} />
        <div class="grow">
          <div class="truncate">{user.display_name || friendlyUserId(user.user_id)}</div>
          <div class="muted small truncate">{user.user_id}</div>
        </div>
        <button class="btn small" disabled={busyUser !== null} onclick={() => run(user.user_id, createRoomWith, 'create the room')} title="Create a new private room and invite them">New room</button>
        <button class="btn primary small" disabled={busyUser !== null} onclick={() => run(user.user_id, openDirectMessage, 'open the conversation')}>Message</button>
      </li>
    {/each}
  </ul>

  {#if searching}<p class="muted"><span class="spinner"></span></p>{/if}
  {#if error}<p class="error-text" role="alert">{error}</p>{/if}
  {#if !searching && !error && query.trim().length >= 2 && results.length === 0 && !typedId}
    <p class="muted">No one found. Try their full <code>@user:server</code> id.</p>
  {/if}
  {#if limited}<p class="muted small">There are more matches — type a bit more to narrow it down.</p>{/if}
</Modal>

<style>
  .results {
    margin: 12px 0 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 4px;
    border-bottom: 1px solid var(--border);
  }
  li:last-child {
    border-bottom: 0;
  }
</style>
