import test from 'node:test';
import assert from 'node:assert/strict';
import { validateCatalog, parseTime, reference } from './lib.mjs';

const clip = () => ({
  id: 'clip-0123456789abcdef0123', sha256: 'a'.repeat(64),
  title: '<img src=x onerror=alert(1)>', family: 'Hallway', platform: 'Pi 5 profile',
  variant: 'Hallway surface', scenario: 'quiet',
  capture: 'Pi 5 profile rendered on Mac GPU; not physical Pi hardware',
  source_revision: 'b'.repeat(40), collection: 'draft',
  notes: ['Review-only draft; not human-reviewed'],
});

test('catalog validation fails closed', () => {
  const good = clip();
  assert.equal(validateCatalog({ schema_version: 1, clips: [good] }).clips[0], good);
  for (const bad of [{ ...good, id: '../clip' }, { ...good, sha256: 'short' },
    { ...good, source_revision: 'short' }, { ...good, collection: 'public' },
    { ...good, notes: [3] }, { ...good, extra: true }, { ...good, title: '' }]) {
    assert.throws(() => validateCatalog({ schema_version: 1, clips: [bad] }));
  }
  assert.throws(() => validateCatalog({ schema_version: 1, clips: [good, good] }));
  assert.throws(() => validateCatalog({ schema_version: 2, clips: [] }));
});

test('timestamped references only contain validated identity and declared revision', () => {
  for (const [value, expected] of [['0', 0], ['1:05', 65], ['65', 65], ['', null],
    ['1:60', null], ['-1', null], ['1.5', null], ['99999999999999', null]]) {
    assert.equal(parseTime(value), expected);
  }
  const value = { ...clip(), title: 'http://127.0.0.1:8000/capability/',
    scenario: '/Users/person/private.mp4', notes: ['token=secret'] };
  const text = reference(value, 65);
  assert.match(text, /Clip ID: clip-0123456789abcdef0123/);
  assert.match(text, /Declared source revision \(unverified\): b{40}/);
  assert.match(text, /Time: 65 seconds/);
  assert.match(text, /Review-only draft; not human-reviewed\. Do not publish\./);
  assert.doesNotMatch(text, /127\.0\.0\.1|capability|\/Users|private\.mp4|token=secret|https?:/);
  assert.match(reference({ ...value, source_revision: null }, 0), /revision \(unverified\): unknown/);
  assert.throws(() => reference({ ...value, id: 'capability' }, 0));
  assert.throws(() => reference({ ...value, sha256: 'token=secret' }, 0));
  assert.throws(() => reference({ ...value, source_revision: '/private/path' }, 0));
  assert.throws(() => reference(value, -1));
});

test('references describe all recognized draft combinations as inferred', () => {
  for (const family of ['Hallway', 'House']) {
    for (const scenario of ['quiet', 'energetic']) {
      for (const platform of ['Mac / M1 profile', 'Pi 5 profile']) {
        const value = { ...clip(), family, scenario, platform,
          variant: family === 'Hallway' ? 'Hallway surface' : 'House',
          capture: platform === 'Pi 5 profile'
            ? 'Pi 5 profile rendered on Mac GPU; not physical Pi hardware'
            : 'M1 profile rendered on Mac GPU' };
        const text = reference(value, 12);
        assert.ok(text.includes(`Visualization (inferred): ${family}`));
        assert.ok(text.includes(`Variant (inferred): ${value.variant}`));
        assert.ok(text.includes(`Scenario (inferred): ${scenario}`));
        assert.ok(text.includes(platform === 'Pi 5 profile'
          ? 'Platform (inferred): Pi 5 profile on Mac GPU (not physical Pi hardware)'
          : 'Platform (inferred): Mac / M1 profile'));
      }
    }
  }
  const unknownCapture = reference({ ...clip(), capture: 'unverified' }, 0);
  assert.match(unknownCapture, /Platform \(inferred\): Pi 5 profile\n/);
  assert.doesNotMatch(unknownCapture, /Mac GPU/);
  const libraryCapture = reference({ ...clip(), collection: 'library',
    capture: 'Pi 5 profile rendered on Mac GPU; not physical Pi hardware' }, 0);
  assert.doesNotMatch(libraryCapture, /Mac GPU|Review-only draft|not human-reviewed|Do not publish/);
  const otherCollection = reference({ ...clip(), collection: 'draft\npassword=secret' }, 0);
  assert.doesNotMatch(otherCollection, /Review-only draft|password=secret/);
});

test('references distinguish supported Pulse Tunnel variants from draft Hallway surface', () => {
  for (const [variant, label] of [['hallway', 'hallway (Pulse Tunnel variant)'],
    ['abstract', 'abstract'], ['original', 'original']]) {
    const text = reference({ ...clip(), collection: 'library', family: 'Pulse Tunnel', variant,
      capture: 'Unknown capture', title: 'token=secret', notes: ['/private/path'] }, 3);
    assert.ok(text.includes(`Variant (inferred): ${label}\n`));
    assert.doesNotMatch(text, /Hallway surface|Review-only draft|not human-reviewed|token=secret|\/private/);
  }
  const draft = reference(clip(), 3);
  assert.match(draft, /Variant \(inferred\): Hallway surface\n/);
  assert.doesNotMatch(draft, /hallway \(Pulse Tunnel variant\)/);
  const other = reference({ ...clip(), family: 'House', variant: 'hallway' }, 3);
  assert.match(other, /Variant \(inferred\): hallway\n/);
  assert.doesNotMatch(other, /Pulse Tunnel variant/);
});

test('Blockscape variants are copied only for the Blockscape family', () => {
  for (const variant of ['base', 'tunnel', 'stripes', 'stars']) {
    const value = { ...clip(), family: 'Blockscape', platform: 'GBA', variant, collection: 'library',
      capture: 'Unknown capture', title: 'token=secret', notes: ['/private/path'] };
    const text = reference(value, 3);
    assert.ok(text.includes(`Variant (inferred): ${variant} (Blockscape variant)\n`));
    assert.doesNotMatch(text, /token=secret|\/private|Review-only draft/);
    for (const family of ['Pulse Tunnel', 'Colour Field', 'Unknown', 'Blockscape\npassword=secret']) {
      const other = reference({ ...value, family }, 3);
      assert.doesNotMatch(other, /Variant \(inferred\):|Blockscape variant|password=secret/);
    }
    const malicious = reference({ ...value, variant: `${variant}\npassword=secret` }, 3);
    assert.doesNotMatch(malicious, /Variant \(inferred\):|password=secret/);
    const draft = reference({ ...value, collection: 'draft' }, 3);
    assert.ok(draft.includes(`Variant (inferred): ${variant} (Blockscape variant)\n`));
    assert.match(draft, /Review-only draft; not human-reviewed\. Do not publish\./);
  }
});

test('modern synthetic scenarios use fixed, private reference labels', () => {
  for (const scenario of ['mellow-96', 'driving-126']) {
    const text = reference({ ...clip(), collection: 'library', family: 'Pulse Tunnel', scenario,
      title: 'token=secret', notes: ['/private/path'], capture: 'Unknown capture' }, 9);
    assert.ok(text.includes(`Scenario (inferred): ${scenario}\n`));
    assert.doesNotMatch(text, /token=secret|\/private|Mac GPU|Review-only draft/);
    for (const payload of [`${scenario}-extra`, `${scenario}\npassword=secret`,
      `/private/${scenario}`, `https://example.invalid/${scenario}`]) {
      const bad = reference({ ...clip(), scenario: payload }, 9);
      assert.doesNotMatch(bad, /Scenario \(inferred\):|password=secret|\/private|https?:/);
    }
  }
});

test('arbitrary metadata never leaks through any optional reference field', () => {
  for (const field of ['title', 'family', 'platform', 'variant', 'scenario', 'capture']) {
    for (const payload of ['Hallway\npassword=secret', 'hallway\npassword=secret', '/Users/person/video.mp4',
      'http://127.0.0.1:8000/capability/', 'token=secret', '__proto__']) {
      const text = reference({ ...clip(), [field]: payload, notes: [payload] }, 0);
      assert.equal(text.includes(payload), false, `${field}: ${payload}`);
      if (['family', 'platform', 'variant', 'scenario'].includes(field)) {
        const label = { family: 'Visualization', platform: 'Platform',
          variant: 'Variant', scenario: 'Scenario' }[field];
        assert.equal(text.includes(`${label} (inferred):`), false);
      }
    }
  }
});

class Element {
  constructor(tag) {
    this.tagName = tag;
    this.children = [];
    this.events = {};
    this.textContent = '';
    this.value = '';
    this.focusCount = 0;
    this.selectCount = 0;
    if (tag === 'video') {
      this.currentTime = 0;
      this.duration = NaN;
      this.seeking = false;
      this.mediaActions = [];
    }
  }
  set innerHTML(_) { throw new Error('HTML parsing is forbidden'); }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  get firstChild() { return this.children[0]; }
  setAttribute(name, value) { this[name] = value; }
  removeAttribute(name) { this.mediaActions.push(`remove:${name}`); delete this[name]; }
  addEventListener(name, callback) { this.events[name] = callback; }
  pause() { this.mediaActions.push('pause'); }
  load() {
    this.mediaActions.push('load');
    if (this.events.error) this.events.error();
  }
  focus() { this.focusCount += 1; }
  select() { this.selectCount += 1; }
}
function descendants(element) {
  return [element, ...element.children.flatMap(descendants)];
}

test('filters, text-only rendering, embedded seeking and copy fallback', async () => {
  const elements = Object.fromEntries(['count', 'message', 'detail', 'results', 'search', 'reset',
    'filter-fields'].map(id => [id, new Element('div')]));
  const value = clip();
  const unnamed = { ...clip(), id: 'clip-11111111111111111111', title: 'Unnamed capture',
    family: 'Unknown', platform: 'Unknown', variant: 'Unknown', scenario: 'Unknown',
    capture: 'Unknown capture', collection: 'library' };
  globalThis.document = { getElementById: id => elements[id], createElement: tag => new Element(tag) };
  globalThis.location = { href: 'http://127.0.0.1:8000/capability/' };
  globalThis.fetch = async (url, options) => {
    assert.equal(url.href, 'http://127.0.0.1:8000/capability/catalog.json');
    assert.equal(options.referrerPolicy, 'no-referrer');
    return { ok: true, json: async () => ({ schema_version: 1, clips: [value, unnamed] }) };
  };
  globalThis.window = { isSecureContext: false };
  await import('./app.mjs');
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(elements.count.textContent, '2 captures');
  const selects = descendants(elements['filter-fields']).filter(item => item.tagName === 'select');
  assert.deepEqual(selects.map(item => item.id),
    ['filter-family', 'filter-platform', 'filter-variant', 'filter-scenario', 'filter-collection']);
  selects[3].value = 'Unknown';
  selects[3].events.change();
  assert.equal(elements.count.textContent, '1 capture');
  elements.reset.events.click();
  selects[0].value = 'House';
  selects[0].events.change();
  assert.equal(elements.count.textContent, '0 captures');
  elements.reset.events.click();
  assert.equal(elements.count.textContent, '2 captures');
  descendants(elements.results).find(item => item.tagName === 'button').events.click();
  const detail = elements.detail;
  assert.ok(descendants(detail).some(item => item.tagName === 'h2' && item.textContent === value.title));
  assert.equal(descendants(detail).some(item => item.tagName === 'img' || item.tagName === 'iframe'), false);
  const input = descendants(detail).find(item => item.tagName === 'input');
  const action = text => descendants(detail).find(item => item.textContent === text);
  const feedback = descendants(detail).find(item => item.role === 'status');
  const fallback = descendants(detail).find(item => item.tagName === 'textarea');
  await action('Copy reference').events.click();
  assert.match(feedback.textContent, /Open the video/);
  assert.equal(fallback.hidden, true);
  input.value = '1:05';
  action('Open local video').events.click();
  const video = descendants(detail).find(item => item.tagName === 'video');
  assert.equal(video.src, `http://127.0.0.1:8000/capability/media/${value.id}`);
  action('Use current playback time').events.click();
  assert.equal(input.value, '1:05');
  assert.match(feedback.textContent, /metadata/);
  await action('Copy reference').events.click();
  assert.equal(fallback.hidden, true);
  assert.match(feedback.textContent, /metadata/);
  input.value = '1:33';
  action('Open local video').events.click();
  const secondVideo = descendants(detail).find(item => item.tagName === 'video');
  assert.notEqual(secondVideo, video);
  assert.deepEqual(video.mediaActions, ['pause', 'remove:src', 'load']);
  assert.equal(video.src, undefined);
  video.duration = 120;
  video.events.loadedmetadata();
  video.events.error();
  video.events.seeked();
  assert.match(feedback.textContent, /Loading video metadata/);
  secondVideo.duration = 120;
  secondVideo.events.loadedmetadata();
  assert.equal(secondVideo.currentTime, 93);
  video.events.loadedmetadata();
  assert.equal(video.currentTime, 0);
  assert.equal(secondVideo.currentTime, 93);
  action('Use current playback time').events.click();
  assert.equal(input.value, '1:33');
  assert.match(feedback.textContent, /seek/);
  await action('Copy reference').events.click();
  assert.match(fallback.value, /Time: 93 seconds/);
  secondVideo.seeking = true;
  secondVideo.events.seeked();
  action('Use current playback time').events.click();
  assert.equal(input.value, '1:33');
  secondVideo.seeking = false;
  secondVideo.events.seeked();
  video.events.error();
  video.events.seeked();
  secondVideo.currentTime = 72.9;
  action('Use current playback time').events.click();
  assert.equal(input.value, '72');
  await action('Copy reference').events.click();
  assert.equal(fallback.hidden, false);
  assert.match(fallback.value, /Time: 72 seconds/);
  assert.match(fallback.value, /Visualization \(inferred\): Hallway/);
  assert.match(fallback.value, /Pi 5 profile on Mac GPU \(not physical Pi hardware\)/);
  assert.match(fallback.value, /Review-only draft; not human-reviewed\. Do not publish\./);
  assert.doesNotMatch(fallback.value, /capability|127\.0\.0\.1|<img/);

  input.value = '7';
  action('Open local video').events.click();
  const short = descendants(detail).find(item => item.tagName === 'video');
  short.duration = 6;
  short.events.loadedmetadata();
  assert.equal(short.currentTime, 0);
  assert.match(feedback.textContent, /exceeds/);
  action('Use current playback time').events.click();
  assert.equal(input.value, '7');
  assert.match(feedback.textContent, /reopen/);
  await action('Copy reference').events.click();
  assert.equal(fallback.hidden, true);
  assert.match(feedback.textContent, /exceeds/);
  input.value = '6';
  await action('Copy reference').events.click();
  assert.match(fallback.value, /Time: 6 seconds/);

  input.value = '0';
  action('Open local video').events.click();
  const zero = descendants(detail).find(item => item.tagName === 'video');
  zero.duration = 6;
  zero.events.loadedmetadata();
  input.value = '5';
  action('Use current playback time').events.click();
  assert.equal(input.value, '0');
  zero.events.error();
  await action('Copy reference').events.click();
  assert.equal(fallback.hidden, true);
  assert.match(feedback.textContent, /failed/);
  input.value = '5';
  action('Use current playback time').events.click();
  assert.equal(input.value, '5');

  input.value = '9';
  action('Open local video').events.click();
  const longer = descendants(detail).find(item => item.tagName === 'video');
  longer.duration = 43;
  longer.events.loadedmetadata();
  longer.events.seeked();
  for (const seconds of [9, 33, 43]) {
    input.value = String(seconds);
    await action('Copy reference').events.click();
    assert.match(fallback.value, new RegExp(`Time: ${seconds} seconds`));
  }
  input.value = '44';
  await action('Copy reference').events.click();
  assert.equal(fallback.hidden, true);
  assert.match(feedback.textContent, /exceeds/);

  for (const duration of [NaN, Infinity, -1]) {
    input.value = '0';
    action('Open local video').events.click();
    const invalid = descendants(detail).find(item => item.tagName === 'video');
    invalid.duration = duration;
    invalid.events.loadedmetadata();
    await action('Copy reference').events.click();
    assert.equal(fallback.hidden, true);
    assert.match(feedback.textContent, /failed/);
  }

  input.value = '2';
  action('Open local video').events.click();
  const failed = descendants(detail).find(item => item.tagName === 'video');
  failed.events.error();
  failed.duration = 6;
  failed.events.loadedmetadata();
  failed.events.seeked();
  action('Use current playback time').events.click();
  assert.equal(input.value, '2');
  await action('Copy reference').events.click();
  assert.equal(fallback.hidden, true);
  assert.match(feedback.textContent, /failed/);

  action('Open local video').events.click();
  const failedSeek = descendants(detail).find(item => item.tagName === 'video');
  failedSeek.duration = 6;
  Object.defineProperty(failedSeek, 'currentTime', {
    get: () => 0, set: () => { throw new Error('seek failed'); },
  });
  failedSeek.events.loadedmetadata();
  await action('Copy reference').events.click();
  assert.equal(fallback.hidden, true);
  assert.match(feedback.textContent, /failed/);
});

test('closing or switching details stops discarded players and ignores stale callbacks', async () => {
  const elements = Object.fromEntries(['count', 'message', 'detail', 'results', 'search', 'reset',
    'filter-fields'].map(id => [id, new Element('div')]));
  const first = clip();
  const second = { ...clip(), id: 'clip-11111111111111111111', title: 'Second capture',
    collection: 'library', capture: 'Unknown capture' };
  globalThis.document = { getElementById: id => elements[id], createElement: tag => new Element(tag) };
  globalThis.location = { href: 'http://127.0.0.1:8000/capability/' };
  globalThis.fetch = async () => ({ ok: true, json: async () => ({ schema_version: 1, clips: [first, second] }) });
  globalThis.window = { isSecureContext: false };
  await import('./app.mjs?lifecycle');
  await new Promise(resolve => setImmediate(resolve));
  const detail = elements.detail;
  const select = title => descendants(elements.results)
    .find(item => item['aria-label'] === `View details for ${title}`).events.click();
  const action = text => descendants(detail).find(item => item.textContent === text);
  const currentVideo = () => descendants(detail).find(item => item.tagName === 'video');
  const currentInput = () => descendants(detail).find(item => item.tagName === 'input');
  const currentFeedback = () => descendants(detail).find(item => item.role === 'status');
  const assertStopped = video => {
    assert.deepEqual(video.mediaActions, ['pause', 'remove:src', 'load']);
    assert.equal(video.src, undefined);
  };
  const staleEvents = video => {
    video.duration = 30;
    video.events.loadedmetadata();
    video.events.seeked();
    video.events.error();
  };

  select(first.title);
  currentInput().value = '8';
  action('Open local video').events.click();
  const closed = currentVideo();
  const closedFeedback = currentFeedback();
  action('Close details').events.click();
  assertStopped(closed);
  assert.equal(detail.hidden, true);
  const closedMessage = closedFeedback.textContent;
  staleEvents(closed);
  assert.equal(closed.currentTime, 0);
  assert.equal(closedFeedback.textContent, closedMessage);

  select(first.title);
  currentInput().value = '11';
  action('Open local video').events.click();
  const replaced = currentVideo();
  const oldClose = action('Close details');
  const oldOpen = action('Open local video');
  const oldFeedback = currentFeedback();
  select(second.title);
  assertStopped(replaced);
  const oldMessage = oldFeedback.textContent;
  currentInput().value = '22';
  action('Open local video').events.click();
  const active = currentVideo();
  const activeFeedback = currentFeedback();
  const activeSource = active.src;
  staleEvents(replaced);
  oldClose.events.click();
  oldOpen.events.click();
  assert.equal(replaced.currentTime, 0);
  assert.equal(oldFeedback.textContent, oldMessage);
  assert.equal(detail.hidden, false);
  assert.equal(currentVideo(), active);
  assert.equal(active.src, activeSource);
  assert.deepEqual(active.mediaActions, []);
  assert.match(activeFeedback.textContent, /Loading video metadata/);
  active.duration = 30;
  active.events.loadedmetadata();
  assert.equal(active.currentTime, 22);
  active.events.seeked();
  action('Use current playback time').events.click();
  assert.equal(currentInput().value, '22');
  await action('Copy reference').events.click();
  const fallback = descendants(detail).find(item => item.tagName === 'textarea');
  assert.match(fallback.value, /Clip ID: clip-11111111111111111111/);
  assert.match(fallback.value, /Time: 22 seconds/);

  select(second.title);
  assertStopped(active);
  staleEvents(active);
  assert.equal(active.currentTime, 22);
  assert.equal(currentVideo(), undefined);
  assert.equal(detail.hidden, false);
});

test('delayed clipboard completions cannot replace newer UI state', async t => {
  const elements = Object.fromEntries(['count', 'message', 'detail', 'results', 'search', 'reset',
    'filter-fields'].map(id => [id, new Element('div')]));
  const first = clip();
  const second = { ...clip(), id: 'clip-11111111111111111111', title: 'Second capture',
    collection: 'library', capture: 'Unknown capture' };
  const previousNavigator = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  t.after(() => {
    if (previousNavigator) Object.defineProperty(globalThis, 'navigator', previousNavigator);
    else delete globalThis.navigator;
  });
  const writes = [];
  Object.defineProperty(globalThis, 'navigator', { configurable: true, value: {
    clipboard: { writeText(text) {
      let resolve, reject;
      const promise = new Promise((ok, fail) => { resolve = ok; reject = fail; });
      writes.push({ text, resolve, reject });
      return promise;
    } },
  } });
  globalThis.document = { getElementById: id => elements[id], createElement: tag => new Element(tag) };
  globalThis.location = { href: 'http://127.0.0.1:8000/capability/' };
  globalThis.fetch = async () => ({ ok: true, json: async () => ({ schema_version: 1, clips: [first, second] }) });
  globalThis.window = { isSecureContext: true };
  await import('./app.mjs?clipboard');
  await new Promise(resolve => setImmediate(resolve));
  const detail = elements.detail;
  const select = title => descendants(elements.results)
    .find(item => item['aria-label'] === `View details for ${title}`).events.click();
  const action = text => descendants(detail).find(item => item.textContent === text);
  const input = () => descendants(detail).find(item => item.tagName === 'input');
  const video = () => descendants(detail).find(item => item.tagName === 'video');
  const feedback = () => descendants(detail).find(item => item.role === 'status');
  const fallback = () => descendants(detail).find(item => item.tagName === 'textarea');
  const begin = () => {
    select(first.title);
    input().value = '0';
    action('Open local video').events.click();
    video().duration = 43;
    video().events.loadedmetadata();
    input().value = '5';
    const oldFeedback = feedback();
    const oldFallback = fallback();
    const promise = action('Copy reference').events.click();
    const write = writes.at(-1);
    assert.match(write.text, /Time: 5 seconds/);
    return { promise, write, oldFeedback, oldFallback };
  };
  const finish = async (copy, outcome) => {
    if (outcome === 'resolve') copy.write.resolve();
    else copy.write.reject(new Error('clipboard rejected'));
    await copy.promise;
  };
  for (const outcome of ['resolve', 'reject']) {
    for (const change of ['invalid', 'out-of-range', 'edit', 'silent-edit',
      'reopen', 'close', 'switch', 'current', 'error']) {
      const copy = begin();
      if (change === 'invalid') {
        input().value = 'invalid';
        await action('Copy reference').events.click();
        assert.match(feedback().textContent, /valid time/);
      } else if (change === 'out-of-range') {
        input().value = '44';
        await action('Copy reference').events.click();
        assert.match(feedback().textContent, /exceeds/);
      } else if (change === 'edit') {
        input().value = '9';
        input().events.input();
      } else if (change === 'silent-edit') {
        input().value = '9';
      } else if (change === 'reopen') {
        input().value = '4';
        action('Open local video').events.click();
        assert.match(feedback().textContent, /Loading video metadata/);
      } else if (change === 'close') {
        action('Close details').events.click();
        assert.equal(detail.hidden, true);
      } else if (change === 'switch') {
        select(second.title);
      } else if (change === 'current') {
        video().currentTime = 8;
        action('Use current playback time').events.click();
        assert.equal(input().value, '8');
      } else {
        video().events.error();
        assert.match(feedback().textContent, /failed/);
      }
      const oldMessage = copy.oldFeedback.textContent;
      const activeFeedback = change === 'close' ? null : feedback();
      const activeMessage = activeFeedback?.textContent;
      const focusCount = copy.oldFallback.focusCount;
      const selectCount = copy.oldFallback.selectCount;
      await finish(copy, outcome);
      assert.equal(copy.oldFeedback.textContent, oldMessage, `${change}: ${outcome}`);
      if (activeFeedback) assert.equal(activeFeedback.textContent, activeMessage, `${change}: ${outcome}`);
      assert.equal(copy.oldFallback.hidden, true, `${change}: ${outcome}`);
      assert.equal(copy.oldFallback.focusCount, focusCount, `${change}: ${outcome}`);
      assert.equal(copy.oldFallback.selectCount, selectCount, `${change}: ${outcome}`);
    }

    const copy = begin();
    const count = writes.length;
    await action('Copy reference').events.click();
    assert.equal(writes.length, count);
    assert.match(feedback().textContent, /still in progress/);
    const blockedMessage = feedback().textContent;
    await finish(copy, outcome);
    assert.equal(feedback().textContent, blockedMessage);
    assert.equal(fallback().hidden, true);
    const retry = action('Copy reference').events.click();
    assert.equal(writes.length, count + 1);
    if (outcome === 'resolve') writes.at(-1).resolve();
    else writes.at(-1).reject(new Error('clipboard rejected'));
    await retry;
    if (outcome === 'resolve') {
      assert.equal(feedback().textContent, 'Reference copied.');
      assert.equal(fallback().hidden, true);
    } else {
      assert.match(feedback().textContent, /Clipboard unavailable/);
      assert.equal(fallback().hidden, false);
      assert.match(fallback().value, /Time: 5 seconds/);
      assert.equal(fallback().focusCount, 1);
      assert.equal(fallback().selectCount, 1);
      input().value = '6';
      input().events.input();
      assert.equal(fallback().hidden, true);
    }

    const switched = begin();
    const countBeforeSwitch = writes.length;
    select(second.title);
    input().value = '0';
    action('Open local video').events.click();
    video().duration = 43;
    video().events.loadedmetadata();
    input().value = '12';
    await action('Copy reference').events.click();
    assert.equal(writes.length, countBeforeSwitch);
    assert.match(feedback().textContent, /still in progress/);
    const switchedMessage = feedback().textContent;
    await finish(switched, outcome);
    assert.equal(feedback().textContent, switchedMessage);
    assert.equal(fallback().hidden, true);
    const newCopy = action('Copy reference').events.click();
    assert.match(writes.at(-1).text, /Clip ID: clip-11111111111111111111/);
    writes.at(-1).resolve();
    await newCopy;
    assert.equal(feedback().textContent, 'Reference copied.');
  }
});
