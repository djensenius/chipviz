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

test('arbitrary metadata never leaks through any optional reference field', () => {
  for (const field of ['title', 'family', 'platform', 'variant', 'scenario', 'capture']) {
    for (const payload of ['Hallway\npassword=secret', '/Users/person/video.mp4',
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
  }
  set innerHTML(_) { throw new Error('HTML parsing is forbidden'); }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  get firstChild() { return this.children[0]; }
  setAttribute(name, value) { this[name] = value; }
  addEventListener(name, callback) { this.events[name] = callback; }
  focus() {}
  select() {}
}
function descendants(element) {
  return [element, ...element.children.flatMap(descendants)];
}

test('filters, text-only rendering, embedded seeking and copy fallback', async () => {
  const elements = Object.fromEntries(['count', 'message', 'detail', 'results', 'search', 'reset',
    'filter-fields'].map(id => [id, new Element('div')]));
  const value = clip();
  globalThis.document = { getElementById: id => elements[id], createElement: tag => new Element(tag) };
  globalThis.location = { href: 'http://127.0.0.1:8000/capability/' };
  globalThis.fetch = async (url, options) => {
    assert.equal(url.href, 'http://127.0.0.1:8000/capability/catalog.json');
    assert.equal(options.referrerPolicy, 'no-referrer');
    return { ok: true, json: async () => ({ schema_version: 1, clips: [value] }) };
  };
  globalThis.window = { isSecureContext: false };
  await import('./app.mjs');
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(elements.count.textContent, '1 capture');
  const selects = descendants(elements['filter-fields']).filter(item => item.tagName === 'select');
  assert.deepEqual(selects.map(item => item.id),
    ['filter-family', 'filter-platform', 'filter-variant', 'filter-scenario', 'filter-collection']);
  selects[0].value = 'House';
  selects[0].events.change();
  assert.equal(elements.count.textContent, '0 captures');
  elements.reset.events.click();
  assert.equal(elements.count.textContent, '1 capture');
  descendants(elements.results).find(item => item.tagName === 'button').events.click();
  const detail = elements.detail;
  assert.ok(descendants(detail).some(item => item.tagName === 'h2' && item.textContent === value.title));
  assert.equal(descendants(detail).some(item => item.tagName === 'img' || item.tagName === 'iframe'), false);
  const input = descendants(detail).find(item => item.tagName === 'input');
  input.value = '1:05';
  descendants(detail).find(item => item.textContent === 'Open local video').events.click();
  const video = descendants(detail).find(item => item.tagName === 'video');
  assert.equal(video.src, `http://127.0.0.1:8000/capability/media/${value.id}`);
  video.events.loadedmetadata();
  assert.equal(video.currentTime, 65);
  video.currentTime = 72.9;
  descendants(detail).find(item => item.textContent === 'Use current playback time').events.click();
  assert.equal(input.value, '72');
  await descendants(detail).find(item => item.textContent === 'Copy reference').events.click();
  const fallback = descendants(detail).find(item => item.tagName === 'textarea');
  assert.equal(fallback.hidden, false);
  assert.match(fallback.value, /Time: 72 seconds/);
  assert.match(fallback.value, /Visualization \(inferred\): Hallway/);
  assert.match(fallback.value, /Pi 5 profile on Mac GPU \(not physical Pi hardware\)/);
  assert.match(fallback.value, /Review-only draft; not human-reviewed\. Do not publish\./);
  assert.doesNotMatch(fallback.value, /capability|127\.0\.0\.1|<img/);
});
