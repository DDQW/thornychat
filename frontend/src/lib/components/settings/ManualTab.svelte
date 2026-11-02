<script lang="ts">
  import { COMMANDS, type CommandCategory } from '../../slash';

  const CATEGORIES: { id: CommandCategory; label: string }[] = [
    { id: 'text', label: 'Text' },
    { id: 'rooms', label: 'Rooms' },
    { id: 'people', label: 'People' },
    { id: 'moderation', label: 'Moderation' },
  ];
</script>

<h2>Writing messages</h2>
<ul class="tips">
  <li><kbd>Enter</kbd> sends, <kbd>Shift</kbd>+<kbd>Enter</kbd> starts a new line, <kbd>↑</kbd> in an empty box edits your last message.</li>
  <li>Markdown works: <code>**bold**</code>, <code>*italic*</code>, <code>`code`</code>, fenced code blocks, lists and quotes. The eye button previews it.</li>
  <li>Type <code>@</code> to mention someone, <code>:</code> and a few letters for an emoji.</li>
  <li>A custom (pack) emoji is written as its <code>:shortcode:</code> and sent as the image, so other clients show it too; the same emoji work as reactions. Click a reply's quote to jump to the original, even when it's further back than what's loaded.</li>
  <li>Paste, drop or attach files; whatever you type becomes the caption of the first one.</li>
  <li>Start a line with <code>//</code> to send a literal single <code>/</code>. An unknown <code>/word</code> is sent as ordinary text.</li>
</ul>

<h2>Keyboard shortcuts</h2>
<dl class="keys">
  <dt><kbd>Ctrl</kbd>+<kbd>K</kbd></dt><dd>Find a room (Enter opens the first match, ↓ moves into the list)</dd>
  <dt><kbd>Alt</kbd>+<kbd>↑</kbd> / <kbd>↓</kbd></dt><dd>Previous / next room</dd>
  <dt><kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>↑</kbd> / <kbd>↓</kbd></dt><dd>Previous / next room with unread messages</dd>
  <dt><kbd>Ctrl</kbd>+<kbd>F</kbd></dt><dd>Search the open room</dd>
  <dt><kbd>Ctrl</kbd>+<kbd>,</kbd></dt><dd>Settings</dd>
  <dt><kbd>Ctrl</kbd>+<kbd>/</kbd></dt><dd>This manual</dd>
  <dt><kbd>Esc</kbd></dt><dd>Close the topmost menu or dialog; cancel a reply or edit</dd>
  <dt><kbd>Tab</kbd> into the messages, then <kbd>↑</kbd> / <kbd>↓</kbd></dt><dd>Move between messages (<kbd>Page Up</kbd>/<kbd>Page Down</kbd> ten at a time, <kbd>Home</kbd>/<kbd>End</kbd> to the oldest/newest loaded, <kbd>Esc</kbd> back to the message box); the menu key opens a message's menu</dd>
  <dt>In the emoji picker</dt><dd><kbd>Enter</kbd> picks the best match, <kbd>↓</kbd> and the arrows move through the grid</dd>
  <dt>In the image viewer</dt><dd><kbd>+</kbd> / <kbd>−</kbd> zoom, <kbd>0</kbd> fits, <kbd>1</kbd> shows actual pixels</dd>
</dl>

<h2>Slash commands</h2>
{#each CATEGORIES as category (category.id)}
  <h3>{category.label}</h3>
  <dl>
    {#each COMMANDS.filter((command) => command.category === category.id) as command (command.name)}
      <dt>
        <code>/{command.name}</code>
        {#each command.aliases as alias (alias)}<code class="alias">/{alias}</code>{/each}
      </dt>
      <dd>
        {#if command.usage}<div class="usage">{command.usage.replace(/^Usage: /, '')}</div>{/if}
        {command.description}
      </dd>
    {/each}
  </dl>
{/each}

<style>
  h3 {
    margin: 14px 0 4px;
    color: var(--muted);
    font-size: 11px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .tips {
    margin: 4px 0;
    padding-left: 20px;
  }
  .tips li {
    margin: 4px 0;
  }
  dl {
    margin: 0;
  }
  dt {
    margin-top: 10px;
    font-weight: 600;
  }
  dd {
    margin: 2px 0 0;
    color: var(--muted);
  }
  .usage {
    color: var(--text);
    font-family: ui-monospace, Consolas, monospace;
    font-size: 12px;
  }
  .alias {
    margin-left: 6px;
    opacity: 0.7;
  }
  code {
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--bg);
    font: 0.92em ui-monospace, Consolas, monospace;
  }
  .keys {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px 16px;
    align-items: baseline;
  }
  .keys dt,
  .keys dd {
    margin: 0;
  }
  .keys dt {
    font-weight: 400;
    white-space: nowrap;
  }
</style>
