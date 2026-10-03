import { backend } from '../api';
import type { UrlPreview } from '../bindings';
import { isSteamLink, isTweetLink } from '../embeds';
import { fire } from '../requests';
import type { SteamAppData, TweetData } from '../types';
import { settings } from './settings.svelte';

type Slot<T> = { state: 'loading' } | { state: 'ready'; value: T } | { state: 'failed' };

/**
 * Link-card data, fetched once per URL and kept for the session.
 *
 * Every fetch here is gated on Settings → Privacy → "Enable link previews":
 * with it off neither the homeserver nor a third-party site (Twitter, Steam,
 * their image CDNs) is contacted to expand a link you received, which would
 * otherwise reveal your IP address and what you're reading.
 */
class PreviewsStore {
  /** OpenGraph data from the homeserver's `preview_url` proxy. */
  og = $state.raw<Record<string, Slot<UrlPreview>>>({});
  tweets = $state.raw<Record<string, Slot<TweetData>>>({});
  steam = $state.raw<Record<string, Slot<SteamAppData>>>({});

  enabled(): boolean {
    return settings.value?.privacy.enable_link_previews ?? false;
  }

  requestOg(url: string): void {
    if (!this.enabled() || this.og[url]) return;
    this.og = { ...this.og, [url]: { state: 'loading' } };
    fire({ type: 'FetchUrlPreview', data: { url } });
  }

  ogFetched(preview: UrlPreview): void {
    this.og = { ...this.og, [preview.url]: { state: 'ready', value: preview } };
  }

  ogFailed(url: string): void {
    this.og = { ...this.og, [url]: { state: 'failed' } };
  }

  async requestTweet(url: string): Promise<void> {
    if (!this.enabled() || this.tweets[url] || !isTweetLink(url)) return;
    this.tweets = { ...this.tweets, [url]: { state: 'loading' } };
    try {
      const tweet = await backend.fetchTweet(url);
      this.tweets = { ...this.tweets, [url]: tweet ? { state: 'ready', value: tweet } : { state: 'failed' } };
    } catch {
      this.tweets = { ...this.tweets, [url]: { state: 'failed' } };
    }
  }

  async requestSteam(url: string): Promise<void> {
    if (!this.enabled() || this.steam[url] || !isSteamLink(url)) return;
    this.steam = { ...this.steam, [url]: { state: 'loading' } };
    try {
      const app = await backend.fetchSteamApp(url);
      this.steam = { ...this.steam, [url]: app ? { state: 'ready', value: app } : { state: 'failed' } };
    } catch {
      this.steam = { ...this.steam, [url]: { state: 'failed' } };
    }
  }

  reset(): void {
    this.og = {};
    this.tweets = {};
    this.steam = {};
  }
}

export const previews = new PreviewsStore();
