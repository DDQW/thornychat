<script lang="ts">
  import { isSteamLink, isTweetLink, videoIn } from '../embeds';
  import { previews } from '../stores/previews.svelte';
  import OgCard from './OgCard.svelte';
  import SteamCard from './SteamCard.svelte';
  import TweetCard from './TweetCard.svelte';
  import VideoEmbed from './VideoEmbed.svelte';

  interface Props {
    urls: string[];
  }
  let { urls }: Props = $props();

  const items = $derived(
    urls.map((url) => {
      const video = videoIn(url);
      if (video) return { kind: 'video' as const, url, video };
      if (isTweetLink(url)) return { kind: 'tweet' as const, url };
      if (isSteamLink(url)) return { kind: 'steam' as const, url };
      return { kind: 'og' as const, url };
    }),
  );

  // Fetch what each card needs, once per URL (every request is gated on the
  // link-preview privacy setting inside the store).
  $effect(() => {
    for (const item of items) {
      if (item.kind === 'tweet') void previews.requestTweet(item.url);
      else if (item.kind === 'steam') void previews.requestSteam(item.url);
      else previews.requestOg(item.url);
    }
  });
</script>

{#each items as item (item.url)}
  {#if item.kind === 'video'}
    <VideoEmbed video={item.video} />
  {:else if item.kind === 'tweet'}
    {@const tweet = previews.tweets[item.url]}
    {#if tweet?.state === 'ready'}
      <TweetCard tweet={tweet.value} url={item.url} />
    {:else if tweet?.state === 'failed'}
      {@const og = previews.og[item.url]}
      {#if og?.state === 'ready'}<OgCard preview={og.value} />{/if}
    {:else if tweet?.state !== 'loading'}
      <!-- previews are off, or not requested yet -->
    {/if}
  {:else if item.kind === 'steam'}
    {@const app = previews.steam[item.url]}
    {#if app?.state === 'ready'}<SteamCard app={app.value} url={item.url} />{/if}
  {:else}
    {@const og = previews.og[item.url]}
    {#if og?.state === 'ready'}<OgCard preview={og.value} />{/if}
  {/if}
{/each}
