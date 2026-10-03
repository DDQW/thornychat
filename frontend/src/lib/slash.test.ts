import { describe, expect, it } from 'vitest';
import { COMMANDS, parse, type Parsed } from './slash';

const msg = (body: string): Parsed => ({ kind: 'message', body, emote: false, markdown: true });
const act = (action: Extract<Parsed, { kind: 'action' }>['action']): Parsed => ({ kind: 'action', action });

describe('slash commands', () => {
  it('plain text passes through', () => {
    expect(parse('hello world')).toEqual(msg('hello world'));
  });

  it('/me is an emote', () => {
    expect(parse('/me waves')).toEqual({ kind: 'message', body: 'waves', emote: true, markdown: true });
  });

  it('a bare /me is a usage error', () => {
    expect(parse('/me').kind).toBe('error');
  });

  it('/method is not a command', () => {
    // A word that merely starts with /me still sends as text.
    expect(parse('/method foo')).toEqual(msg('/method foo'));
  });

  it('/plain skips markdown', () => {
    expect(parse('/plain **bold**')).toEqual({ kind: 'message', body: '**bold**', emote: false, markdown: false });
  });

  it('/shrug with and without text', () => {
    expect(parse('/shrug')).toEqual(msg('¯\\_(ツ)_/¯'));
    expect(parse('/shrug hi')).toEqual(msg('¯\\_(ツ)_/¯ hi'));
  });

  it('a double slash escapes', () => {
    expect(parse('//me literally')).toEqual(msg('/me literally'));
  });

  it('room and people actions', () => {
    expect(parse('/join #a:b.com')).toEqual(act({ type: 'join', room: '#a:b.com' }));
    expect(parse('/j #a:b.com')).toEqual(act({ type: 'join', room: '#a:b.com' }));
    expect(parse('/knock #a:b.com')).toEqual(act({ type: 'knock', room: '#a:b.com' }));
    expect(parse('/leave')).toEqual(act({ type: 'leave' }));
    expect(parse('/part')).toEqual(act({ type: 'leave' }));
    expect(parse('/invite @u:b.com')).toEqual(act({ type: 'invite', user: '@u:b.com' }));
    expect(parse('/dm @u:b.com')).toEqual(act({ type: 'dm', user: '@u:b.com' }));
    expect(parse('/query @u:b.com')).toEqual(act({ type: 'dm', user: '@u:b.com' }));
  });

  it('/kick splits the user from the reason', () => {
    expect(parse('/kick @u:b.com being rude')).toEqual(act({ type: 'kick', user: '@u:b.com', reason: 'being rude' }));
    expect(parse('/ban @u:b.com')).toEqual(act({ type: 'ban', user: '@u:b.com', reason: null }));
  });

  it('/ignore and /unignore are account-wide and take the first token only', () => {
    expect(parse('/ignore @u:b.com')).toEqual(act({ type: 'ignore', user: '@u:b.com' }));
    expect(parse('/unignore @u:b.com')).toEqual(act({ type: 'unignore', user: '@u:b.com' }));
    // There is no reason argument to keep: m.ignored_user_list records a bare user id.
    expect(parse('/ignore @u:b.com because')).toEqual(act({ type: 'ignore', user: '@u:b.com' }));
  });

  it('bad arguments are usage errors', () => {
    for (const input of ['/kick', '/kick notauser', '/join notaroom', '/knock notaroom', '/ignore', '/unignore notauser']) {
      expect(parse(input).kind, input).toBe('error');
    }
  });

  it('an unknown command sends as text', () => {
    expect(parse('/wobble on')).toEqual(msg('/wobble on'));
  });

  it('commands are case-insensitive', () => {
    expect(parse('/LEAVE')).toEqual(act({ type: 'leave' }));
  });

  it('every command in the catalog actually parses', () => {
    // Catches a COMMANDS entry with no matching case in parse(): an unmatched
    // name falls through to the unknown-command branch, which echoes the input
    // back verbatim as a message. A real case never produces that exact echo
    // (it errors, acts, or sends a transformed body).
    for (const spec of COMMANDS) {
      expect(parse(`/${spec.name}`), `/${spec.name}`).not.toEqual(msg(`/${spec.name}`));
      for (const alias of spec.aliases) {
        expect(parse(`/${alias}`), `/${alias}`).not.toEqual(msg(`/${alias}`));
      }
    }
  });
});
