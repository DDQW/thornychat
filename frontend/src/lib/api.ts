// The page's only door to the Rust shell. In the app this is Tauri's IPC; in a
// plain browser (`npm run dev` without `cargo tauri dev`) it is the mock in
// ./mock.ts, so the UI can be built and looked at without a homeserver.

import type { ClientCommand, ClientEvent } from './bindings';
import type {
  AppInfo,
  LoginFlows,
  OwnProfile,
  ProfileFile,
  RestoreOutcome,
  SessionInfo,
  Settings,
  SpellStatus,
  SteamAppData,
  ThemeConfig,
  TweetData,
} from './types';

export interface AttachmentMeta {
  roomId: string;
  requestId: string;
  caption: string | null;
  mentionedUserIds: string[];
  replyToEventId: string | null;
}

export interface Backend {
  attachEvents(onEvent: (event: ClientEvent) => void): Promise<void>;
  sessionRestore(): Promise<RestoreOutcome>;
  discoverHomeserver(homeserver: string): Promise<LoginFlows>;
  loginPassword(homeserver: string, username: string, password: string): Promise<SessionInfo>;
  loginSso(homeserver: string, identityProviderId: string | null): Promise<SessionInfo>;
  dispatch(command: ClientCommand): Promise<void>;
  ownProfile(): Promise<OwnProfile>;
  sendAttachment(file: File, meta: AttachmentMeta): Promise<void>;

  appInfo(): Promise<AppInfo>;
  getSettings(): Promise<Settings>;
  setSettings(settings: Settings): Promise<Settings>;
  themePresets(): Promise<ThemeConfig[]>;
  fetchTweet(url: string): Promise<TweetData | null>;
  fetchSteamApp(url: string): Promise<SteamAppData | null>;
  openExternal(url: string): Promise<void>;
  saveMedia(mxcUrl: string, suggestedName: string): Promise<boolean>;
  loadProfileFile(file: ProfileFile): Promise<string | null>;
  saveProfileFile(file: ProfileFile, contents: string): Promise<void>;
  autostartEnabled(): Promise<boolean>;
  setAutostart(enabled: boolean): Promise<void>;
  readLog(): Promise<string>;
  clearLogs(): Promise<string>;
  quit(): Promise<void>;
  /** The Windows speller: whether it is usable (and warm), and in which language. */
  spellStatus(): Promise<SpellStatus>;
  /** One verdict per word, in order: `true` for misspelled. */
  spellCheck(words: string[]): Promise<boolean[]>;
  spellSuggest(word: string): Promise<string[]>;
  /** The fix autocorrect may apply, when the speller is confident of one. */
  spellCorrection(word: string): Promise<string | null>;
  /** Adds a word to the user's Windows dictionary. */
  spellAdd(word: string): Promise<void>;
  /** Game-activity connector: the emote body to post (e.g. "plays Half-Life"). */
  onConnectorEmote(handler: (body: string) => void): Promise<() => void>;

  /** URL the webview can load for an `mxc://` URI, served by the shell. */
  mediaUrl(mxcUrl: string, mimeHint?: string): string;
  /** A Lanczos3-upscaled copy of an `mxc://` image with this long edge, made by the shell; null where there is none. */
  upscaledUrl(mxcUrl: string, longEdge: number): string | null;
  twemojiUrl(codepoints: string): string;
  webImageUrl(url: string): string;
}

export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

async function createTauriBackend(): Promise<Backend> {
  const { invoke, Channel } = await import('@tauri-apps/api/core');
  const { listen } = await import('@tauri-apps/api/event');

  // The window is created with `use_https_scheme`, so WebView2 serves custom
  // schemes as https://<scheme>.localhost; other platforms use <scheme>://localhost.
  const mediaBase = navigator.userAgent.includes('Windows') ? 'https://tcmedia.localhost' : 'tcmedia://localhost';
  const mediaUrl = (mxcUrl: string, mimeHint?: string): string => {
    const match = /^mxc:\/\/([^/]+)\/([^/?#]+)$/.exec(mxcUrl);
    if (!match) return '';
    const query = mimeHint ? `?mime=${encodeURIComponent(mimeHint)}` : '';
    return `${mediaBase}/mxc/${match[1]}/${match[2]}${query}`;
  };

  return {
    async attachEvents(onEvent) {
      const channel = new Channel<ClientEvent>();
      channel.onmessage = onEvent;
      await invoke('attach_events', { onEvent: channel });
    },
    sessionRestore: () => invoke('session_restore'),
    discoverHomeserver: (homeserver) => invoke('discover_homeserver', { homeserver }),
    loginPassword: (homeserver, username, password) => invoke('login_password', { homeserver, username, password }),
    loginSso: (homeserver, identityProviderId) =>
      invoke('login_sso', { homeserver, identityProviderId }),
    dispatch: (command) => invoke('dispatch', { command }),
    ownProfile: () => invoke('own_profile'),
    async sendAttachment(file, meta) {
      // A raw body keeps a 50 MB file from becoming a 200 MB JSON array; the
      // rest rides in headers, which must be ASCII, hence encodeURIComponent.
      const headers: Record<string, string> = {
        'x-room-id': encodeURIComponent(meta.roomId),
        'x-request-id': meta.requestId,
        'x-filename': encodeURIComponent(file.name || 'file'),
        'x-mime': encodeURIComponent(file.type || 'application/octet-stream'),
        'x-caption': encodeURIComponent(meta.caption ?? ''),
        'x-mentions': encodeURIComponent(JSON.stringify(meta.mentionedUserIds)),
        'x-reply-to': encodeURIComponent(meta.replyToEventId ?? ''),
      };
      await invoke('send_attachment', new Uint8Array(await file.arrayBuffer()), { headers });
    },

    appInfo: () => invoke('app_info'),
    getSettings: () => invoke('get_settings'),
    setSettings: (settings) => invoke('set_settings', { new: settings }),
    themePresets: () => invoke('theme_presets'),
    fetchTweet: (url) => invoke('fetch_tweet', { url }),
    fetchSteamApp: (url) => invoke('fetch_steam_app', { url }),
    openExternal: (url) => invoke('open_external', { url }),
    saveMedia: (mxcUrl, suggestedName) => invoke('save_media', { mxcUrl, suggestedName }),
    loadProfileFile: (file) => invoke('load_profile_file', { file }),
    saveProfileFile: (file, contents) => invoke('save_profile_file', { file, contents }),
    autostartEnabled: () => invoke('autostart_enabled'),
    setAutostart: (enabled) => invoke('set_autostart', { enabled }),
    readLog: () => invoke('read_log'),
    clearLogs: () => invoke('clear_logs'),
    quit: () => invoke('quit_app'),
    spellStatus: () => invoke('spell_status'),
    spellCheck: (words) => invoke('spell_check', { words }),
    spellSuggest: (word) => invoke('spell_suggest', { word }),
    spellCorrection: (word) => invoke('spell_correction', { word }),
    spellAdd: (word) => invoke('spell_add', { word }),
    onConnectorEmote: (handler) => listen<string>('connector-emote', (event) => handler(event.payload)),

    mediaUrl,
    upscaledUrl(mxcUrl, longEdge) {
      const base = mediaUrl(mxcUrl);
      return base ? `${base}?upscale=${Math.round(longEdge)}` : null;
    },
    twemojiUrl: (codepoints) => `${mediaBase}/twemoji/${codepoints}.svg`,
    webImageUrl: (url) => `${mediaBase}/web?u=${encodeURIComponent(url)}`,
  };
}

async function createBackend(): Promise<Backend> {
  if (inTauri) return createTauriBackend();
  const { createMockBackend } = await import('./mock');
  return createMockBackend();
}

export const backend: Backend = await createBackend();
