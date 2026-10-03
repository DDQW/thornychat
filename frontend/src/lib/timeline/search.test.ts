import { describe, expect, it } from 'vitest';
import type { TimelineItem, TimelineItemContent } from '../bindings';
import { matchesQuery } from './search';

function item(content: TimelineItemContent): TimelineItem {
  return {
    event_id: '$e',
    sender: '@a:b',
    sender_display_name: null,
    sender_avatar_url: null,
    timestamp_ms: 0,
    content,
    formatted_body: null,
    shield: null,
    reactions: [],
    thread_root: null,
    thread_reply_count: null,
    read_by: [],
    in_reply_to: null,
    edited: false,
    send_failed: null,
  };
}

describe('matchesQuery', () => {
  it('matches message text case-insensitively', () => {
    expect(matchesQuery(item({ type: 'Text', data: 'Deploy is Green' }), 'deploy', 'Alice')).toBe(true);
    expect(matchesQuery(item({ type: 'Emote', data: 'waves hello' }), 'HELLO', 'Alice')).toBe(true);
    expect(matchesQuery(item({ type: 'Text', data: 'unrelated' }), 'deploy', 'Alice')).toBe(false);
  });

  it('matches the sender’s name too', () => {
    expect(matchesQuery(item({ type: 'Text', data: 'hi' }), 'alice', 'Alice')).toBe(true);
  });

  it('matches captions, file names and sticker text', () => {
    expect(matchesQuery(item({ type: 'Image', data: { url: 'mxc://a/b', caption: 'Sunset', width: null, height: null } }), 'sunset', 'x')).toBe(true);
    expect(matchesQuery(item({ type: 'File', data: { url: 'mxc://a/b', filename: 'report.pdf', caption: null } }), 'report', 'x')).toBe(true);
    expect(matchesQuery(item({ type: 'Video', data: { url: 'mxc://a/b', filename: 'a.mp4', caption: 'demo', mimetype: null, thumbnail_url: null } }), 'demo', 'x')).toBe(true);
    expect(matchesQuery(item({ type: 'Sticker', data: { url: 'mxc://a/b', body: 'thorn-wave', width: null, height: null } }), 'wave', 'x')).toBe(true);
  });

  it('never matches dividers, redactions or system lines', () => {
    for (const content of [{ type: 'DateDivider', data: 'Today' }, { type: 'NewMessagesDivider' }, { type: 'Redacted' }, { type: 'MembershipChange', data: 'alice joined' }] as TimelineItemContent[]) {
      expect(matchesQuery(item(content), 'today', 'alice'), content.type).toBe(false);
    }
  });

  it('an empty or blank query matches nothing', () => {
    expect(matchesQuery(item({ type: 'Text', data: 'anything' }), '', 'a')).toBe(false);
    expect(matchesQuery(item({ type: 'Text', data: 'anything' }), '   ', 'a')).toBe(false);
  });
});
