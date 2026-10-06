<script module lang="ts">
  /** Unsent drafts survive switching rooms (in memory — they're not worth persisting). */
  const drafts = new Map<string, string>();
</script>

<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { editMessage, runSlashAction, sendMessage, sendSticker, setTyping } from '../actions';
  import { backend } from '../api';
  import { loadEmojiData, type EmojiData } from '../emoji';
  import { formatBytes, truncate } from '../format';
  import { searchMembers } from '../members';
  import { nameOf } from '../people';
  import { enrichHtml } from '../richtext';
  import { sanitizeFormattedBody } from '../sanitize';
  import { CommandError, expectOutcome, newRequestId } from '../requests';
  import { parse } from '../slash';
  import { DraftSpeller, EMPTY_SCAN, editKind, markSegments, scanDraft, type Draft, type Word } from '../spell';
  import { composer } from '../stores/composer.svelte';
  import { emoji as emojiStore } from '../stores/emoji.svelte';
  import { rooms } from '../stores/rooms.svelte';
  import { session } from '../stores/session.svelte';
  import { settings } from '../stores/settings.svelte';
  import { spelling } from '../stores/spelling.svelte';
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

  // Keep the draft as it changes. (The textarea grows with its content by
  // itself, see `field-sizing` in the styles.)
  $effect(() => {
    drafts.set(roomId, text);
  });

  // --- context: reply / edit / injected text ---
  $effect(() => {
    const target = composer.editing;
    if (target?.content.type === 'Text') {
      text = target.content.data;
      mentions = [];
      speller.reset();
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
    speller.interrupt();
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
    return searchMembers(members, mentionQuery, 9)
      .filter((member) => member.user_id !== session.info?.user_id)
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
    speller.interrupt();
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

  // --- spelling ---
  // Words go to the Windows speller in the shell; WebView2's own checker marks
  // nothing in this app. Marks are drawn by an overlay behind the transparent
  // textarea, so the native attribute is only switched on as a fallback when
  // our speller is unavailable (two sets of squiggles would be worse than one).
  const spellOn = $derived(settings.value?.spellcheck.enabled ?? true);
  const autocorrectOn = $derived(spellOn && (settings.value?.spellcheck.autocorrect ?? false));
  const speller = new DraftSpeller();
  let overlay = $state<HTMLDivElement>();
  /** The textarea's text box (padding included, scrollbar excluded), which the overlay copies. */
  let box = $state({ width: 0, height: 0 });

  $effect(() => {
    if (spellOn) spelling.loadStatus();
  });

  const caret = $derived((void caretVersion, void text, textarea?.selectionStart ?? text.length));

  /** Where live mentions sit: a display name is never a typo. */
  const mentionSpans = $derived.by(() => {
    const spans: [number, number][] = [];
    for (const mention of mentions) {
      const needle = `@${mention.displayName}`;
      for (let at = text.indexOf(needle); at >= 0; at = text.indexOf(needle, at + needle.length)) spans.push([at, at + needle.length]);
    }
    return spans;
  });

  const scan = $derived.by(() => {
    void spelling.version;
    if (!spellOn) return EMPTY_SCAN;
    const inMention = (word: Word) => mentionSpans.some(([start, end]) => word.rawStart < end && word.rawEnd > start);
    return scanDraft(text, caret, (word) => spelling.verdict(word), inMention);
  });
  $effect(() => {
    if (scan.unknown.length > 0) spelling.request(scan.unknown);
  });
  const segments = $derived(scan.marks.length > 0 ? markSegments(text, scan.marks) : []);

  /** The speller's suggestions for the last marked word the caret visited. */
  let suggested = $state.raw<{ word: string; suggestions: string[] } | null>(null);
  $effect(() => {
    const word = scan.target?.core;
    if (!word || suggested?.word === word) return;
    backend.spellSuggest(word).then(
      (suggestions) => (suggested = { word, suggestions }),
      () => {},
    );
  });
  const flagged = $derived(scan.target && suggested?.word === scan.target.core ? { word: scan.target, suggestions: suggested.suggestions } : null);

  /** Puts the caret at `position` once the new text is in the textarea. */
  function placeCaret(position: number, refocus = false) {
    void tick().then(() => {
      if (refocus) textarea?.focus();
      textarea?.setSelectionRange(position, position);
      caretVersion++;
    });
  }

  /** Swaps a marked word for `replacement`, unless the draft has moved on under it. */
  function replaceWord(word: Word, replacement: string) {
    const current = textarea?.value ?? text;
    if (current.slice(word.start, word.end) !== word.core) return;
    speller.interrupt();
    text = current.slice(0, word.start) + replacement + current.slice(word.end);
    placeCaret(word.start + replacement.length, true);
  }

  async function addToDictionary(word: string) {
    try {
      await spelling.add(word);
    } catch (error) {
      ui.error(`Couldn't add “${word}” to the dictionary: ${String(error)}`);
    }
    suggested = null;
    textarea?.focus();
  }

  /** Every edit: undo an autocorrect on the Backspace right after it, or autocorrect the word a space just finished. */
  function onedit(event: Event) {
    const area = textarea;
    if (!area || !(event instanceof InputEvent)) return;
    const kind = editKind(event);
    const current = (): Draft => ({ text: area.value, caret: area.selectionStart });
    const undone = speller.afterEdit(kind, current());
    if (undone) {
      text = undone.text;
      placeCaret(undone.caret);
    } else if (kind === 'word-end' && autocorrectOn) {
      speller.autocorrect(current(), (word) => backend.spellCorrection(word), current).then(
        (fixed) => {
          if (!fixed) return;
          text = fixed.text;
          placeCaret(fixed.caret);
        },
        () => {},
      );
    }
  }

  /** Right-click on a marked word: its suggestions, in place of the native menu. */
  function oncontextmenu(event: MouseEvent) {
    const inside = (rect: DOMRect) => event.clientX >= rect.left && event.clientX <= rect.right && event.clientY >= rect.top && event.clientY <= rect.bottom;
    const mark = overlay ? [...overlay.querySelectorAll<HTMLElement>('mark')].find((element) => [...element.getClientRects()].some(inside)) : undefined;
    const word = mark && scan.marks.find((candidate) => candidate.start === Number(mark.dataset.start));
    if (!word) return;
    event.preventDefault();
    event.stopPropagation();
    const at = { x: event.clientX, y: event.clientY };
    backend.spellSuggest(word.core).then(
      (suggestions) => {
        const fixes = suggestions.length
          ? suggestions.map((suggestion) => ({ label: suggestion, onselect: () => replaceWord(word, suggestion) }))
          : [{ label: 'No suggestions', disabled: true, onselect: () => {} }];
        ui.menu = { ...at, items: [...fixes, { label: 'Add to dictionary', separator: true, onselect: () => void addToDictionary(word.core) }] };
      },
      () => {},
    );
  }

  /** Keeps the overlay the same size as the textarea's text box. */
  function measure() {
    if (!textarea) return;
    const { clientWidth: width, clientHeight: height } = textarea;
    if (width !== box.width || height !== box.height) box = { width, height };
  }
  $effect(() => {
    const area = textarea;
    if (!area) return;
    const observer = new ResizeObserver(measure);
    observer.observe(area);
    return () => observer.disconnect();
  });

  function syncScroll() {
    if (overlay && textarea) overlay.scrollTop = textarea.scrollTop;
  }
  $effect(() => {
    void segments;
    syncScroll();
  });

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
        speller.reset();
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
    speller.reset();
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
    speller.reset();
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
  // The parser is loaded on first use: a sixth of the startup bundle, for a
  // toggle most people never touch (sending is converted in the shell).
  let markdown = $state.raw<typeof import('marked') | null>(null);
  $effect(() => {
    if (previewing && !markdown) void import('marked').then((module) => (markdown = module));
  });
  const previewHtml = $derived.by(() => {
    if (!previewing || !text.trim() || !markdown) return '';
    const dirty = markdown.marked.parse(text, { async: false, gfm: true, breaks: true }) as string;
    return enrichHtml(sanitizeFormattedBody(dirty, { mediaUrl: (mxc) => backend.mediaUrl(mxc) }), {
      twemojiUrl: (codepoints) => backend.twemojiUrl(codepoints),
      customEmoji: (shortcode) => {
        const custom = emojiStore.resolve(shortcode);
        return custom ? { url: backend.mediaUrl(custom.mxc_url), shortcode: custom.shortcode } : null;
      },
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

  {#if flagged}
    <div class="context spell" role="group" aria-label="Spelling suggestions for {flagged.word.core}">
      <span class="muted">{flagged.suggestions.length > 0 ? 'Did you mean' : `No suggestions for “${flagged.word.core}”`}</span>
      {#each flagged.suggestions as suggestion (suggestion)}
        <button class="btn small" onmousedown={(e) => e.preventDefault()} onclick={() => replaceWord(flagged.word, suggestion)}>{suggestion}</button>
      {/each}
      <button class="btn small ghost" onmousedown={(e) => e.preventDefault()} onclick={() => addToDictionary(flagged.word.core)}>Add to dictionary</button>
    </div>
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

    <div class="editor grow">
      {#if segments.length > 0}
        <div class="spell-marks" bind:this={overlay} aria-hidden="true" style:width="{box.width}px" style:height="{box.height}px">{#each segments as segment (segment.start)}{#if segment.typo}<mark data-start={segment.start}>{segment.text}</mark>{:else}{segment.text}{/if}{/each}{'\u200b'}</div>
      {/if}
      <textarea
        bind:this={textarea}
        bind:value={text}
        rows="1"
        placeholder={staged.length ? 'Add a caption…' : 'Message… (@mention, markdown supported, / for commands)'}
        spellcheck={spellOn && spelling.status?.available === false}
        aria-label="Message"
        {onkeydown}
        {onpaste}
        {oncontextmenu}
        oninput={(e) => { noteTyping(); caretVersion++; inlineError = ''; onedit(e); }}
        onclick={() => caretVersion++}
        onkeyup={() => caretVersion++}
        onscroll={syncScroll}
        onblur={stopTyping}
      ></textarea>
    </div>

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
  .spell {
    flex-wrap: wrap;
  }
  .editor {
    position: relative;
    display: flex;
  }
  /* The marks sit behind the (transparent) textarea, laid out exactly like its text. */
  .spell-marks {
    position: absolute;
    top: 0;
    left: 0;
    padding: 6px 4px;
    overflow: hidden;
    color: transparent;
    line-height: 1.4;
    white-space: pre-wrap;
    overflow-wrap: break-word;
    pointer-events: none;
  }
  .spell-marks mark {
    background: none;
    color: transparent;
    text-decoration: underline wavy var(--danger);
    text-decoration-skip-ink: none;
    text-underline-offset: 3px;
  }
  /* Grows with its content without the old `height: auto` + `scrollHeight`
     round trip, which collapsed the composer for one forced layout per keystroke
     and let the timeline above clamp its scroll position — losing the bottom. */
  textarea {
    position: relative;
    z-index: 1;
    field-sizing: content;
    width: 100%;
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
