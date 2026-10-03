<script lang="ts">
  import { backend } from '../api';
  import { compactNumber } from '../format';
  import { enrichHtml, plainTextToHtml } from '../richtext';
  import type { TweetData } from '../types';
  import Avatar from './Avatar.svelte';
  import TweetCard from './TweetCard.svelte';

  interface Props {
    tweet: TweetData;
    url: string;
    nested?: boolean;
  }
  let { tweet, url, nested = false }: Props = $props();

  const html = $derived(plainTextToHtml(tweet.text, { twemojiUrl: (c) => backend.twemojiUrl(c) }));
  const photos = $derived((tweet.media?.photos ?? []).slice(0, 4));
  const videoThumbs = $derived((tweet.media?.videos ?? []).map((v) => v.thumbnail_url).filter((u): u is string => !!u).slice(0, 1));
  const stats = $derived(
    [
      ['replies', tweet.replies],
      ['reposts', tweet.retweets],
      ['likes', tweet.likes],
      ['views', tweet.views],
    ].filter((entry): entry is [string, number] => typeof entry[1] === 'number'),
  );
</script>

<article class="tweet" class:nested>
  <header>
    {#if tweet.author.avatar_url}
      <img class="avatar" src={backend.webImageUrl(tweet.author.avatar_url)} alt="" width="32" height="32" loading="lazy" />
    {:else}
      <Avatar name={tweet.author.name} size={32} />
    {/if}
    <div class="who">
      <span class="name-line">{tweet.author.name}{#if tweet.author.verification?.verified}<span class="verified" title="Verified">✔</span>{/if}</span>
      <span class="muted small">@{tweet.author.screen_name}</span>
    </div>
    {#if !nested}<span class="x muted" aria-hidden="true">𝕏</span>{/if}
  </header>

  <div class="text">{@html html}</div>

  {#if photos.length > 0 || videoThumbs.length > 0}
    <div class="media" class:single={photos.length + videoThumbs.length === 1}>
      {#each photos as photo (photo.url)}
        <img src={backend.webImageUrl(photo.url)} alt="" loading="lazy" />
      {/each}
      {#each videoThumbs as thumb (thumb)}
        <img src={backend.webImageUrl(thumb)} alt="Video thumbnail" loading="lazy" />
      {/each}
    </div>
  {/if}

  {#if tweet.quote}
    <TweetCard tweet={tweet.quote} {url} nested />
  {/if}

  {#if !nested}
    <footer class="muted small">
      {#each stats as [label, value] (label)}<span>{compactNumber(value)} {label}</span>{/each}
      <a class="open" href={url} data-external>Open</a>
    </footer>
  {/if}
</article>

<style>
  .tweet {
    width: min(460px, 100%);
    margin-top: 6px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .tweet.nested {
    width: 100%;
    margin-top: 8px;
    background: transparent;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .avatar {
    border-radius: 50%;
  }
  .who {
    display: grid;
    flex: 1 1 auto;
    min-width: 0;
    line-height: 1.2;
  }
  .name-line {
    font-weight: 700;
  }
  .verified {
    margin-left: 4px;
    color: #1d9bf0;
    font-size: 12px;
  }
  .text {
    margin: 8px 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .text :global(img.twemoji) {
    width: 1.2em;
    height: 1.2em;
    vertical-align: -0.25em;
  }
  .media {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 3px;
    border-radius: var(--radius-sm);
    overflow: hidden;
  }
  .media.single {
    grid-template-columns: 1fr;
  }
  .media img {
    width: 100%;
    max-height: 280px;
    object-fit: cover;
  }
  footer {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    margin-top: 8px;
  }
  .open {
    margin-left: auto;
  }
</style>
