<script module lang="ts">
  /** Unsent drafts survive switching rooms (in memory — they're not worth persisting). */
  const drafts = new Map<string, string>();
</script>

<script lang="ts">
  import { marked } from 'marked';
  import { tick, untrack } from 'svelte';
  import { editMessage, runSlashAction, sendMessage, sendSticker, setTyping } from '../actions';
  import { backend } from '../api';
  import { loadEmojiData, type EmojiData } from '../emoji';
  import { formatBytes, truncate } from '../format';
  import { nameOf } from '../people';
  import { enrichHtml } from '../richtext';
  import { sanitizeFormattedBody } from '../sanitize';
  import { CommandError, expectOutcome, newRequestId } from '../requests';
  import { parse } from '../slash';
  import { composer } from '../stores/composer.svelte';
  import { emoji as emojiStore } from '../stores/emoji.svelte';
  import { rooms } from '../stores/rooms.svelte';
  import { session } from '../stores/session.svelte';
  import { settings } from '../stores/settings.svelte';
  import { timelines } from '../stores/timelines.svelte';
  import { ui } from '../stores/ui.svelte';
  import EmojiPicker, { type Pick } from './EmojiPicker.svelte';
  import Icon from './Icon.svelte';

  interface Props {
    roomId: string;
  }
  let { roomId }: Props = $props();

  interface Staged {
    file: File;
    /** Object URL for an image thumbnail. */
    preview: string | null;
  }

  let text = $state(untrack(() => drafts.get(roomId) ?? ''));
  let textarea = $state<HTMLTextAreaElement>();
  let fileInput = $state<HTMLInputElement>();
  let staged = $state<Staged[]>([]);
  let mentions = $state<{ userId: string; displayName: string }[]>([]);
  let inlineError = $state('');
  let busy = $state(false);
  let previewing = $state(false);
  let pickerAnchor = $state<DOMRect | null>(null);
  let selected = $state(0);
  let emojiData = $state<EmojiData | null>(null);

  const editing = $derived(composer.editing);
  const replyTo = $derived(composer.replyTo);
  const members = $derived(rooms.members[roomId] ?? []);

  // Keep the draft as it changes; the textarea grows with its content.
  $effect(() => {
    drafts.set(roomId, text);
    void textarea;
    void tick().then(autosize);
  });

  function autosize() {
    if (!textarea) return;
    textarea.style.height = 'auto';
    textarea.style.height = `${Math.min(textarea.scrollHeight, window.innerHeight * 0.4)}px`;
  }

  // --- context: reply / edit / injected text ---
  $effect(() => {
    const target = composer.editing;
    if (target?.content.type === 'Text') {
      text = target.content.data;
      mentions = [];
      void tick().then(() => textarea?.focus());
    }
  });
  $effect(() => {
    if (composer.replyTo) void tick().then(() => textarea?.focus());
  });

  let lastInject = 0;
  $effect(() => {
    const request = composer.inject;
    if (!request || request.nonce === lastInject) return;
    lastInject = request.nonce;
    insertAtCaret(request.text);
    if (request.mention && !mentions.some((m) => m.userId === request.mention!.userId)) {
      mentions = [...mentions, request.mention];
    }
  });

  function insertAtCaret(insert: string) {
    const area = textarea;
    const start = area?.selectionStart ?? text.length;
    const end = area?.selectionEnd ?? text.length;
    text = text.slice(0, start) + insert + text.slice(end);
    void tick().then(() => {
      textarea?.focus();
      const caret = start + insert.length;
      textarea?.setSelectionRange(caret, caret);
    });
  }

  // --- autocomplete: @mentions and :emoji: ---
  /** The word being typed at the caret, if it starts with the given trigger. */
  function activeToken(trigger: '@' | ':'): string | null {
    const caret = textarea?.selectionStart ?? text.length;
    const before = text.slice(0, caret);
    const match = trigger === '@' ? /(?:^|\s)@([^\s@]*)$/.exec(before) : /(?:^|\s):([a-z0-9_+-]{2,})$/i.exec(before);
    return match ? (match[1] ?? '') : null;
  }

  let caretVersion = $state(0);
  const mentionQuery = $derived((void caretVersion, void text, activeToken('@')));
  const emojiQuery = $derived((void caretVersion, void text, activeToken(':')));

  const mentionMatches = $derived.by(() => {
    if (mentionQuery === null) return [];
    const needle = mentionQuery.toLowerCase();
    return members
      .filter((member) => member.user_id !== session.info?.user_id)
      .filter((member) => `${member.display_name} ${member.user_id}`.toLowerCase().includes(needle))
      .slice(0, 8);
  });

  const emojiMatches = $derived.by(() => {
    if (emojiQuery === null) return [];
    const needle = emojiQuery.toLowerCase();
    const custom = [...emojiStore.byShortcode.values()]
      .filter((e) => e.shortcode.toLowerCase().includes(needle))
      .slice(0, 6)
      .map((e) => ({ kind: 'custom' as const, key: `custom:${e.mxc_url}`, label: `:${e.shortcode}:`, mxc: e.mxc_url, insert: `:${e.shortcode}: ` }));
    const unicode = emojiData
      ? [...emojiData.shortcodes.entries()]
          .filter(([code]) => code.includes(needle))
          .slice(0, 8 - custom.length)
          .map(([code, e]) => ({ kind: 'unicode' as const, key: `u:${code}`, label: `:${code}:`, char: e.char, insert: `${e.char} ` }))
      : [];
    return [...custom, ...unicode];
  });

  $effect(() => {
    if (emojiQuery !== null && !emojiData) void loadEmojiData().then((data) => (emojiData = data));
  });
  $effect(() => {
    void mentionMatches.length;
    void emojiMatches.length;
    selected = 0;
  });

  const menuOpen = $derived(mentionMatches.length > 0 || emojiMatches.length > 0);

  function replaceToken(trigger: '@' | ':', insert: string) {
    const caret = textarea?.selectionStart ?? text.length;
    const before = text.slice(0, caret);
    const at = trigger === '@' ? before.lastIndexOf('@') : before.lastIndexOf(':');
    text = text.slice(0, at) + insert + text.slice(caret);
    void tick().then(() => {
      const position = at + insert.length;
      textarea?.focus();
      textarea?.setSelectionRange(position, position);
      caretVersion++;
    });
  }

  function pickMention(member: { user_id: string; display_name: string }) {
    const displayName = member.display_name || nameOf(member.user_id, roomId);
    replaceToken('@', `@${displayName} `);
    if (!mentions.some((m) => m.userId === member.user_id)) mentions = [...mentions, { userId: member.user_id, displayName }];
  }

  function pickEmojiMatch(match: (typeof emojiMatches)[number]) {
    replaceToken(':', match.insert);
    if (match.kind === 'unicode') emojiStore.recordUse(match.char);
  }

  // --- attachments ---
  export function addFiles(files: Iterable<File>) {
    const added: Staged[] = [];
    for (const file of files) {
      if (file.size === 0) continue;
      added.push({ file, preview: file.type.startsWith('image/') ? URL.createObjectURL(file) : null });
    }
    if (added.length > 0) {
      staged = [...staged, ...added];
      void tick().then(() => textarea?.focus());
    }
  }

  function unstage(entry: Staged) {
    if (entry.preview) URL.revokeObjectURL(entry.preview);
    staged = staged.filter((candidate) => candidate !== entry);
  }

  function clearStaged() {
    for (const entry of staged) if (entry.preview) URL.revokeObjectURL(entry.preview);
    staged = [];
  }

  function onpaste(event: ClipboardEvent) {
    const files = [...(event.clipboardData?.files ?? [])];
    if (files.length === 0) return; // plain text pastes natively
    event.preventDefault();
    addFiles(files);
  }

  // --- typing indicator ---
  let typingTimer: ReturnType<typeof setTimeout> | null = null;
  let typingSent = false;
  function noteTyping() {
    if (!text.trim()) return stopTyping();
    if (!typingSent) {
      typingSent = true;
      setTyping(roomId, true);
    }
    if (typingTimer) clearTimeout(typingTimer);
    typingTimer = setTimeout(stopTyping, 5000);
  }
  function stopTyping() {
    if (typingTimer) clearTimeout(typingTimer);
    typingTimer = null;
    if (typingSent) {
      typingSent = false;
      setTyping(roomId, false);
    }
  }
  $effect(() => () => stopTyping());

  // --- sending ---
  /** Mentions whose `@Name` text is still in the draft; a deleted one must not still ping. */
  function liveMentions(): string[] {
    return mentions.filter((m) => text.includes(`@${m.displayName}`)).map((m) => m.userId);
  }

  async function submit() {
    if (busy) return;
    inlineError = '';
    const body = text;

    if (editing && editing.event_id) {
      if (!body.trim()) return;
      busy = true;
      try {
        await editMessage(roomId, editing.event_id, body);
        composer.clear();
        text = '';
      } catch (error) {
        inlineError = `Couldn't edit the message: ${String(error)}`;
      } finally {
        busy = false;
      }
      return;
    }

    if (staged.length > 0) return void (await submitAttachments(body));
    if (!body.trim()) return;

    const parsed = parse(body);
    if (parsed.kind === 'error') {
      inlineError = parsed.message;
      return;
    }

    const snapshot = { text: body, mentions: [...mentions], reply: replyTo };
    const mentionIds = liveMentions();
    text = '';
    mentions = [];
    composer.clear();
    stopTyping();

    try {
      if (parsed.kind === 'action') {
        const done = await runSlashAction(roomId, parsed.action);
        if (done) ui.toast(done, 'success', 3000);
      } else {
        await sendMessage(roomId, parsed.body, {
          mentionedUserIds: mentionIds,
          replyTo: snapshot.reply?.event_id ?? null,
          emote: parsed.emote,
          markdown: parsed.markdown,
        });
      }
    } catch (error) {
      // Put the draft back so nothing typed is lost.
      text = snapshot.text;
      mentions = snapshot.mentions;
      inlineError = String(error);
    }
  }

  async function submitAttachments(caption: string) {
    busy = true;
    const files = [...staged];
    const mentionIds = liveMentions();
    const reply = replyTo?.event_id ?? null;
    text = '';
    mentions = [];
    composer.clear();
    clearStagedKeepingFiles();
    let failures = 0;
    // The typed text is the caption of the first attachment (MSC2530).
    for (const [index, entry] of files.entries()) {
      const requestId = newRequestId();
      const { outcome, abort } = expectOutcome(requestId);
      // A failure to even start the upload rejects `outcome` with nobody awaiting it.
      outcome.catch(() => {});
      try {
        await backend.sendAttachment(entry.file, {
          roomId,
          requestId,
          caption: index === 0 && caption.trim() ? caption.trim() : null,
          mentionedUserIds: index === 0 ? mentionIds : [],
          replyToEventId: index === 0 ? reply : null,
        });
        await outcome;
      } catch (error) {
        abort(new CommandError(String(error)));
        failures++;
        ui.error(`Couldn't send ${entry.file.name}: ${String(error)}`);
      } finally {
        if (entry.preview) URL.revokeObjectURL(entry.preview);
      }
    }
    busy = false;
    if (failures > 0 && failures === files.length) inlineError = 'None of the files could be sent.';
  }

  /** Empties the chip row without revoking previews (the in-flight sends still hold them). */
  function clearStagedKeepingFiles() {
    staged = [];
  }

  function onkeydown(event: KeyboardEvent) {
    if (menuOpen) {
      const count = mentionMatches.length || emojiMatches.length;
      if (event.key === 'ArrowDown') return void (event.preventDefault(), (selected = (selected + 1) % count));
      if (event.key === 'ArrowUp') return void (event.preventDefault(), (selected = (selected + count - 1) % count));
      if (event.key === 'Enter' || event.key === 'Tab') {
        event.preventDefault();
        if (mentionMatches.length) pickMention(mentionMatches[selected]!);
        else pickEmojiMatch(emojiMatches[selected]!);
        return;
      }
      if (event.key === 'Escape') {
        event.stopPropagation();
        // Dismiss by moving the trigger out of scope.
        text = text.replace(/[@:][^\s]*$/, (m) => m.slice(1));
        return;
      }
    }
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      void submit();
    } else if (event.key === 'Escape' && (composer.replyTo || composer.editing)) {
      event.stopPropagation();
      if (composer.editing) text = '';
      composer.clear();
    } else if (event.key === 'ArrowUp' && text === '' && !event.shiftKey) {
      // Up in an empty composer edits your last message, like most chat apps.
      const last = timelineLastOwnText();
      if (last) {
        event.preventDefault();
        composer.edit(last);
      }
    }
  }

  function timelineLastOwnText() {
    const items = timelines.get(roomId)?.items;
    return items?.findLast((item) => item.sender === session.info?.user_id && item.content.type === 'Text' && item.event_id !== null) ?? null;
  }

  // --- emoji picker ---
  function onpick(pick: Pick) {
    pickerAnchor = null;
    if (pick.kind === 'unicode') insertAtCaret(pick.char);
    else if (pick.kind === 'custom') insertAtCaret(`:${pick.emoji.shortcode}: `);
    else {
      sendSticker(roomId, pick.sticker).catch((error: unknown) => ui.error(`Couldn't send the sticker: ${String(error)}`));
    }
  }

  // --- markdown preview (what recipients will see, approximately) ---
  const previewHtml = $derived.by(() => {
    if (!previewing || !text.trim()) return '';
    const dirty = marked.parse(text, { async: false, gfm: true, breaks: true }) as string;
    return enrichHtml(sanitizeFormattedBody(dirty, { mediaUrl: (mxc) => backend.mediaUrl(mxc) }), {
      twemojiUrl: (codepoints) => backend.twemojiUrl(codepoints),
    });
  });

  const canSend = $derived(!busy && (text.trim().length > 0 || staged.length > 0));
</script>

<div class="composer">
  {#if replyTo}
    <div class="context">
      <Icon name="reply" size={14} />
      <span class="grow truncate">Replying to <strong>{nameOf(replyTo.sender, roomId, replyTo.sender_display_name)}</strong>
        <span class="muted">{replyTo.content.type === 'Text' || replyTo.content.type === 'Emote' ? truncate(replyTo.content.data, 80) : `[${replyTo.content.type.toLowerCase()}]`}</span></span>
      <button class="icon-btn" onclick={() => composer.clear()} aria-label="Cancel reply"><Icon name="x" size={14} /></button>
    </div>
  {/if}
  {#if editing}
    <div class="context">
      <Icon name="edit" size={14} />
      <span class="grow">Editing message — Enter to save, Esc to cancel</span>
      <button class="icon-btn" onclick={() => { composer.clear(); text = ''; }} aria-label="Cancel edit"><Icon name="x" size={14} /></button>
    </div>
  {/if}

  {#if staged.length > 0}
    <div class="chips">
      {#each staged as entry (entry)}
        <div class="chip">
          {#if entry.preview}<img src={entry.preview} alt="" />{:else}<Icon name="file" size={20} />{/if}
          <div class="info"><div class="truncate">{entry.file.name}</div><div class="muted small">{formatBytes(entry.file.size)}</div></div>
          <button class="icon-btn" onclick={() => unstage(entry)} aria-label="Remove {entry.file.name}"><Icon name="x" size={14} /></button>
        </div>
      {/each}
      <span class="muted small hint">Type a caption for the first file, then press Enter.</span>
    </div>
  {/if}

  {#if previewing && previewHtml}
    <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitized -->
    <div class="preview">{@html previewHtml}</div>
  {/if}

  {#if menuOpen}
    <ul class="autocomplete" role="listbox">
      {#if mentionMatches.length}
        {#each mentionMatches as member, index (member.user_id)}
          <li role="option" aria-selected={index === selected}>
            <button class:on={index === selected} onmousedown={(e) => { e.preventDefault(); pickMention(member); }}>
              <strong>{member.display_name || nameOf(member.user_id, roomId)}</strong> <span class="muted small">{member.user_id}</span>
            </button>
          </li>
        {/each}
      {:else}
        {#each emojiMatches as match, index (match.key)}
          <li role="option" aria-selected={index === selected}>
            <button class:on={index === selected} onmousedown={(e) => { e.preventDefault(); pickEmojiMatch(match); }}>
              {#if match.kind === 'custom'}<img class="em" src={backend.mediaUrl(match.mxc)} alt="" />{:else}<span class="em">{match.char}</span>{/if}
              {match.label}
            </button>
          </li>
        {/each}
      {/if}
    </ul>
  {/if}

  <div class="bar">
    <input bind:this={fileInput} type="file" multiple hidden onchange={(e) => { addFiles(e.currentTarget.files ?? []); e.currentTarget.value = ''; }} />
    <button class="icon-btn" onclick={() => fileInput?.click()} aria-label="Attach files" title="Attach files"><Icon name="paperclip" /></button>

    <textarea
      bind:this={textarea}
      bind:value={text}
      rows="1"
      class="grow"
      placeholder={staged.length ? 'Add a caption…' : 'Message… (@mention, markdown supported, / for commands)'}
      spellcheck={settings.value?.spellcheck.enabled ?? true}
      aria-label="Message"
      {onkeydown}
      {onpaste}
      oninput={() => { noteTyping(); caretVersion++; inlineError = ''; }}
      onclick={() => caretVersion++}
      onkeyup={() => caretVersion++}
      onblur={stopTyping}
    ></textarea>

    <button class="icon-btn" class:active={previewing} onclick={() => (previewing = !previewing)} aria-label="Preview markdown" aria-pressed={previewing} title="Preview markdown"><Icon name="eye" /></button>
    <button class="icon-btn" onclick={(e) => (pickerAnchor = e.currentTarget.getBoundingClientRect())} aria-label="Emoji and stickers" title="Emoji and stickers"><Icon name="smile" /></button>
    <button class="btn primary send" onclick={submit} disabled={!canSend} aria-label="Send"><Icon name="send" size={16} /></button>
  </div>

  {#if inlineError}<p class="error-text small err" role="alert">{inlineError}</p>{/if}
</div>

{#if pickerAnchor}
  <EmojiPicker anchor={pickerAnchor} {onpick} onclose={() => (pickerAnchor = null)} stickers />
{/if}

<style>
  .composer {
    position: relative;
    padding: 0 16px 14px;
  }
  .context {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-bottom: 0;
    border-radius: var(--radius) var(--radius) 0 0;
    background: var(--surface);
    font-size: 12px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 8px;
    border: 1px solid var(--border);
    border-bottom: 0;
    border-radius: var(--radius) var(--radius) 0 0;
    background: var(--surface);
  }
  .chip {
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: 240px;
    padding: 4px 4px 4px 8px;
    border-radius: var(--radius-sm);
    background: var(--surface-strong);
  }
  .chip img {
    width: 36px;
    height: 36px;
    border-radius: 4px;
    object-fit: cover;
  }
  .chip .info {
    min-width: 0;
  }
  .hint {
    flex-basis: 100%;
  }
  .preview {
    max-height: 30vh;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-bottom: 0;
    background: var(--surface);
    overflow: auto;
  }
  .preview :global(p) {
    margin: 0 0 6px;
  }
  .preview :global(pre) {
    padding: 8px;
    border-radius: var(--radius-sm);
    background: var(--bg);
    overflow-x: auto;
  }
  .preview :global(img.twemoji) {
    width: 1.3em;
    height: 1.3em;
    vertical-align: -0.3em;
  }
  .bar {
    display: flex;
    align-items: flex-end;
    gap: 4px;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .bar:focus-within {
    border-color: var(--accent);
  }
  .context + .bar,
  .chips + .bar,
  .preview + .bar {
    border-radius: 0 0 var(--radius) var(--radius);
  }
  textarea {
    max-height: 40vh;
    min-height: 32px;
    padding: 6px 4px;
    border: 0;
    background: transparent;
    outline: none;
    resize: none;
    line-height: 1.4;
  }
  .send {
    width: 36px;
    height: 32px;
    padding: 0;
  }
  .err {
    margin: 6px 4px 0;
  }
  .autocomplete {
    position: absolute;
    right: 16px;
    bottom: 100%;
    left: 16px;
    z-index: 5;
    max-height: 260px;
    margin: 0 0 4px;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-strong);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.35);
    list-style: none;
    overflow-y: auto;
  }
  .autocomplete button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    text-align: left;
  }
  .autocomplete button.on,
  .autocomplete button:hover {
    background: var(--hover);
  }
  .em {
    width: 22px;
    height: 22px;
    font-size: 18px;
    line-height: 22px;
    text-align: center;
    object-fit: contain;
  }
</style>
