<script lang="ts">
  import { redactMessage, retrySend, toggleReaction } from '../actions';
  import { backend } from '../api';
  import type { TimelineItem } from '../bindings';
  import { extractLinks } from '../embeds';
  import { formatDateTime, formatTime, nameHue, plural, truncate } from '../format';
  import { memberMenu } from '../memberMenu';
  import { avatarOf, memberMap, nameOf } from '../people';
  import { composer } from '../stores/composer.svelte';
  import { settings } from '../stores/settings.svelte';
  import { session } from '../stores/session.svelte';
  import { ui } from '../stores/ui.svelte';
  import Avatar from './Avatar.svelte';
  import EmojiPicker, { type Pick } from './EmojiPicker.svelte';
  import FileCard from './FileCard.svelte';
  import Icon from './Icon.svelte';
  import LinkCards from './LinkCards.svelte';
  import MediaImage from './MediaImage.svelte';
  import MessageBody from './MessageBody.svelte';
  import Reactions from './Reactions.svelte';
  import VideoAttachment from './VideoAttachment.svelte';

  interface Props {
    item: TimelineItem;
    roomId: string;
    /** Same sender as the row above, shortly after: no header. */
    continuation: boolean;
    onjump: (eventId: string) => void;
  }
  let { item, roomId, continuation, onjump }: Props = $props();

  const myId = $derived(session.info?.user_id ?? '');
  const mine = $derived(item.sender === myId);
  const eventId = $derived(item.event_id);
  const pending = $derived(eventId === null && mine);
  const name = $derived(nameOf(item.sender, roomId, item.sender_display_name));
  const avatar = $derived(avatarOf(item.sender, roomId) ?? item.sender_avatar_url);
  const content = $derived(item.content);
  const canModerate = $derived((memberMap(roomId)?.get(myId)?.power_level ?? 0) >= 50);
  const canEdit = $derived(mine && eventId !== null && content.type === 'Text');
  const canDelete = $derived(eventId !== null && content.type !== 'Redacted' && (mine || canModerate));

  const plainText = $derived(content.type === 'Text' || content.type === 'Emote' ? content.data : null);
  const links = $derived(content.type === 'Text' && settings.value?.privacy.enable_link_previews ? extractLinks(content.data) : []);
  const highlighted = $derived(
    !mine &&
      plainText !== null &&
      ((!!item.formatted_body && item.formatted_body.includes(myId)) || plainText.includes(myId) || (!!session.profile?.display_name && plainText.includes(`@${session.profile.display_name}`))),
  );
  const flash = $derived(ui.highlightEvent !== null && ui.highlightEvent === eventId);
  const readers = $derived(item.read_by.filter((id) => id !== myId).slice(0, 6));

  let pickerAnchor = $state<DOMRect | null>(null);
  let confirmingDelete = $state(false);

  function react(pick: Pick) {
    pickerAnchor = null;
    if (!eventId) return;
    const key = pick.kind === 'unicode' ? pick.char : pick.kind === 'custom' ? pick.emoji.mxc_url : null;
    if (key) toggleReaction(roomId, eventId, key).catch((error: unknown) => ui.error(`Couldn't react: ${String(error)}`));
  }

  async function remove() {
    confirmingDelete = false;
    if (!eventId) return;
    try {
      await redactMessage(roomId, eventId);
    } catch (error) {
      ui.error(`Couldn't delete the message: ${String(error)}`);
    }
  }

  function copyText() {
    if (plainText !== null) void navigator.clipboard.writeText(plainText).then(() => ui.toast('Copied', 'success', 1500));
  }

  function openMenu(event: MouseEvent) {
    if ((event.target as Element).closest('a, input, textarea, video')) return;
    ui.showMenu(event, [
      ...(eventId ? [{ label: 'Reply', onselect: () => composer.reply(item) }] : []),
      ...(canEdit ? [{ label: 'Edit', onselect: () => composer.edit(item) }] : []),
      ...(plainText !== null ? [{ label: 'Copy text', onselect: copyText }] : []),
      ...(canDelete ? [{ label: 'Delete…', danger: true, separator: true, onselect: () => (confirmingDelete = true) }] : []),
    ]);
  }

  function openSender(event: MouseEvent) {
    ui.showMenu(event, memberMenu(item.sender, name));
  }
</script>

<div
  class="msg"
  class:continuation
  class:pending
  class:highlighted
  class:flash
  data-event-id={eventId}
  style:--hue={nameHue(item.sender)}
  oncontextmenu={openMenu}
  role="article"
>
  <div class="gutter">
    {#if !continuation && content.type !== 'Emote'}
      <button class="plain" onclick={openSender} aria-label="{name}: user menu"><Avatar src={avatar} {name} seed={item.sender} size={36} /></button>
    {:else}
      <time class="stamp" title={formatDateTime(item.timestamp_ms)}>{formatTime(item.timestamp_ms)}</time>
    {/if}
  </div>

  <div class="main">
    {#if !continuation && content.type !== 'Emote'}
      <div class="head">
        <button class="plain name" onclick={openSender}>{name}</button>
        <time class="muted small" title={formatDateTime(item.timestamp_ms)}>{formatTime(item.timestamp_ms)}</time>
        {#if item.shield}
          <span class="shield {item.shield.type.toLowerCase()}" title={item.shield.data}><Icon name={item.shield.type === 'Red' ? 'shield-alert' : 'shield'} size={14} /></span>
        {/if}
      </div>
    {/if}

    {#if item.in_reply_to}
      {@const reply = item.in_reply_to}
      <button class="reply plain" onclick={() => onjump(reply.event_id)} title="Jump to the quoted message">
        <Icon name="reply" size={13} />
        {#if reply.sender}<span class="reply-name">{reply.sender}</span>{/if}
        {#if reply.image_url}<img src={backend.mediaUrl(reply.image_url)} alt="" loading="lazy" />{/if}
        <span class="truncate muted">{reply.snippet}</span>
      </button>
    {/if}

    {#if content.type === 'Text'}
      <div class="content">
        <MessageBody text={content.data} formatted={item.formatted_body} />
        {#if item.edited}<span class="edited muted small" title="This message was edited">(edited)</span>{/if}
      </div>
      {#if links.length > 0}<LinkCards urls={links} />{/if}
    {:else if content.type === 'Emote'}
      <div class="content emote">
        <span class="star">*</span>
        <button class="plain name" onclick={openSender}>{name}</button>
        <MessageBody text={content.data} formatted={item.formatted_body} />
        {#if item.edited}<span class="edited muted small">(edited)</span>{/if}
      </div>
    {:else if content.type === 'Image'}
      <MediaImage mxcUrl={content.data.url} name={content.data.caption ?? 'image'} width={content.data.width} height={content.data.height} />
      {#if content.data.caption}<div class="content caption"><MessageBody text={content.data.caption} /></div>{/if}
    {:else if content.type === 'Sticker'}
      <MediaImage mxcUrl={content.data.url} name={content.data.body} width={content.data.width} height={content.data.height} max={160} />
    {:else if content.type === 'File'}
      <FileCard mxcUrl={content.data.url} filename={content.data.filename} />
      {#if content.data.caption}<div class="content caption"><MessageBody text={content.data.caption} /></div>{/if}
    {:else if content.type === 'Video'}
      <VideoAttachment mxcUrl={content.data.url} filename={content.data.filename} mimetype={content.data.mimetype} thumbnailUrl={content.data.thumbnail_url} />
      {#if content.data.caption}<div class="content caption"><MessageBody text={content.data.caption} /></div>{/if}
    {:else if content.type === 'Redacted'}
      <div class="content muted"><em>Message deleted</em></div>
    {/if}

    {#if eventId && item.reactions.length > 0}
      <Reactions {roomId} {eventId} reactions={item.reactions} />
    {/if}

    {#if item.thread_reply_count}
      <div class="thread muted small"><Icon name="reply" size={12} /> {plural(item.thread_reply_count, 'reply', 'replies')} in thread</div>
    {/if}

    {#if item.send_failed}
      <div class="failed small" role="alert">
        <Icon name="info" size={13} />
        <span>Not sent: {truncate(item.send_failed.error, 120)}</span>
        {#if item.send_failed.is_recoverable}
          <button class="btn small" onclick={() => retrySend(roomId)}>Retry</button>
        {/if}
      </div>
    {/if}

    {#if readers.length > 0}
      <div class="readers" title="Read by {readers.map((id) => nameOf(id, roomId)).join(', ')}">
        {#each readers as reader (reader)}<Avatar src={avatarOf(reader, roomId)} name={nameOf(reader, roomId)} seed={reader} size={14} />{/each}
      </div>
    {/if}
  </div>

  {#if eventId && content.type !== 'Redacted'}
    <div class="actions" class:open={confirmingDelete || pickerAnchor !== null}>
      {#if confirmingDelete}
        <span class="small">Delete this message?</span>
        <button class="btn danger small" onclick={remove}>Delete</button>
        <button class="btn small" onclick={() => (confirmingDelete = false)}>Cancel</button>
      {:else}
        <button class="icon-btn" title="Add reaction" aria-label="Add reaction" onclick={(e) => (pickerAnchor = e.currentTarget.getBoundingClientRect())}><Icon name="smile" size={16} /></button>
        <button class="icon-btn" title="Reply" aria-label="Reply" onclick={() => composer.reply(item)}><Icon name="reply" size={16} /></button>
        {#if canEdit}<button class="icon-btn" title="Edit" aria-label="Edit" onclick={() => composer.edit(item)}><Icon name="edit" size={16} /></button>{/if}
        {#if canDelete}<button class="icon-btn" title="Delete" aria-label="Delete" onclick={() => (confirmingDelete = true)}><Icon name="trash" size={16} /></button>{/if}
        <button class="icon-btn" title="More" aria-label="More" onclick={(e) => { const r = e.currentTarget.getBoundingClientRect(); openMenu(new MouseEvent('contextmenu', { clientX: r.left, clientY: r.bottom })); }}><Icon name="more" size={16} /></button>
      {/if}
    </div>
  {/if}
</div>

{#if pickerAnchor}
  <EmojiPicker anchor={pickerAnchor} onpick={react} onclose={() => (pickerAnchor = null)} />
{/if}

<style>
  .msg {
    position: relative;
    display: flex;
    gap: 12px;
    padding: 6px 16px 6px 12px;
  }
  .msg:not(.continuation) {
    margin-top: 8px;
  }
  .msg:hover,
  .msg:focus-within {
    background: var(--hover);
  }
  .msg.pending {
    opacity: 0.6;
  }
  .msg.highlighted {
    background: var(--accent-wash);
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .msg.flash {
    animation: flash 1.6s ease-out;
  }
  @keyframes flash {
    from {
      background: var(--accent-wash);
    }
  }
  .gutter {
    display: flex;
    flex: 0 0 40px;
    justify-content: center;
    padding-top: 2px;
  }
  .stamp {
    visibility: hidden;
    color: var(--muted);
    font-size: 10px;
    line-height: 20px;
    white-space: nowrap;
  }
  .msg:hover .stamp {
    visibility: visible;
  }
  .main {
    flex: 1 1 auto;
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .plain {
    padding: 0;
    border: 0;
    background: none;
    text-align: left;
  }
  .name:hover {
    text-decoration: underline;
  }
  .shield {
    display: inline-flex;
    align-self: center;
  }
  .shield.red {
    color: var(--danger);
  }
  .shield.grey {
    color: var(--muted);
  }
  .content {
    line-height: 1.45;
  }
  .content.emote {
    display: flex;
    flex-wrap: wrap;
    gap: 0 5px;
    align-items: baseline;
    color: var(--emote);
  }
  .content.emote :global(.body) {
    display: inline;
  }
  .edited {
    margin-left: 4px;
  }
  .caption {
    margin-top: 4px;
  }
  .reply {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    margin-bottom: 2px;
    color: var(--muted);
    font-size: 12px;
  }
  .reply:hover {
    color: var(--text);
  }
  .reply-name {
    font-weight: 600;
  }
  .reply img {
    height: 20px;
    border-radius: 3px;
  }
  .thread {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-top: 4px;
    color: var(--accent);
  }
  .failed {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 4px;
    color: var(--danger);
  }
  .readers {
    display: flex;
    justify-content: flex-end;
    gap: 2px;
    margin-top: 2px;
  }
  .actions {
    position: absolute;
    top: -14px;
    right: 16px;
    display: none;
    align-items: center;
    gap: 2px;
    padding: 2px 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-strong);
    box-shadow: 0 4px 14px rgb(0 0 0 / 0.3);
  }
  .msg:hover .actions,
  .msg:focus-within .actions,
  .actions.open {
    display: flex;
  }
</style>
