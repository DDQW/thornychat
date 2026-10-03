// Shapes of the values the Rust shell returns that are not part of the
// client-core protocol (those are generated into ./bindings). They mirror
// crates/desktop: config/*.rs, bridge.rs, embeds/*.rs.

export interface SessionInfo {
  user_id: string;
  device_id: string | null;
  homeserver: string;
}

export type RestoreOutcome =
  | { status: 'none' }
  | { status: 'started'; session: SessionInfo }
  | { status: 'failed'; message: string; retryable: boolean };

export interface LoginFlows {
  supports_password: boolean;
  supports_sso: boolean;
  sso_providers: { id: string; name: string }[];
}

export interface OwnProfile {
  display_name: string | null;
  avatar_url: string | null;
}

/** The small per-profile files the shell stores for the page (see commands.rs). */
export type ProfileFile = 'emoji_usage' | 'stickers' | 'last_room';

export interface AppInfo {
  version: string;
  profile: string;
  logging_enabled: boolean;
}

export interface ThemeConfig {
  name: string;
  dark: boolean;
  background: string;
  surface: string;
  surface_strong: string;
  text: string;
  muted_text: string;
  accent: string;
  accent_text: string;
  success: string;
  danger: string;
  emote: string;
  font_family: string | null;
  ui_scale: number;
  corner_radius: number;
}

export type LogLevel = 'off' | 'error' | 'warn' | 'info' | 'debug' | 'trace';

export interface Settings {
  theme: ThemeConfig;
  chat: { show_membership_events: boolean; hide_members: boolean };
  privacy: { send_read_receipts: boolean; send_typing_notifications: boolean; enable_link_previews: boolean };
  encryption: { encrypt_direct_messages: boolean; encrypt_rooms: boolean };
  connectors: {
    steam_enabled: boolean;
    gog_enabled: boolean;
    epic_enabled: boolean;
    announce_stop: boolean;
    poll_interval_secs: number;
  };
  spellcheck: { enabled: boolean; autocorrect: boolean };
  log: { level: LogLevel };
}

export interface TweetData {
  text: string;
  author: {
    name: string;
    screen_name: string;
    avatar_url?: string | null;
    verification?: { verified: boolean } | null;
  };
  replies?: number | null;
  retweets?: number | null;
  likes?: number | null;
  views?: number | null;
  created_timestamp?: number | null;
  media?: { photos: { url: string }[]; videos: { thumbnail_url?: string | null }[] } | null;
  quote?: TweetData | null;
}

export interface SteamAppData {
  name: string;
  short_description?: string | null;
  header_image?: string | null;
  is_free: boolean;
  platforms?: { windows: boolean; mac: boolean; linux: boolean } | null;
  price_overview?: { discount_percent: number; initial_formatted: string; final_formatted: string } | null;
  release_date?: { coming_soon: boolean; date: string } | null;
}
