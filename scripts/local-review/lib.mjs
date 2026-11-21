const ID = /^clip-[0-9a-f]{20}$/;
const HASH = /^[0-9a-f]{64}$/;
const REVISION = /^[0-9a-f]{40}$/;
const FIELDS = ['id', 'sha256', 'title', 'family', 'platform', 'variant',
  'scenario', 'capture', 'source_revision', 'collection', 'notes'];

// These fixed labels are the only optional metadata copied into references.
const CONTEXT = [
  ['family', 'Visualization', new Map([
    ['Hallway', 'Hallway'], ['House', 'House'], ['Signal Field', 'Signal Field'],
    ['Spectrum Volume', 'Spectrum Volume'], ['Pulse Tunnel', 'Pulse Tunnel'],
    ['Lines', 'Lines'], ['Static', 'Static'], ['Object', 'Object'],
    ['Colour Field', 'Colour Field'], ['Starfield', 'Starfield'],
    ['Blockscape', 'Blockscape'], ['Tile Scroll', 'Tile Scroll'],
    ['Diamond Scroll', 'Diamond Scroll'], ['PETSCII Meter Field', 'PETSCII Meter Field'],
    ['Perspective Meters', 'Perspective Meters'], ['Tile Meters', 'Tile Meters'],
    ['Static diagnostic', 'Static diagnostic'],
  ])],
  ['platform', 'Platform', new Map([
    ['Mac / M1 profile', 'Mac / M1 profile'], ['Pi 5 profile', 'Pi 5 profile'],
    ['Game Boy', 'Game Boy'], ['NES', 'NES'], ['C64', 'C64'], ['GBA', 'GBA'],
    ['N64', 'N64'], ['SNES', 'SNES'], ['Genesis', 'Genesis'], ['Master System', 'Master System'],
  ])],
  ['variant', 'Variant', new Map([
    ['Hallway surface', 'Hallway surface'], ['House', 'House'], ['Original', 'Original'],
    ['hallway', 'hallway'], ['abstract', 'abstract'], ['original', 'original'],
    ['spectral', 'spectral'], ['held', 'held'], ['hard-cuts', 'hard-cuts'],
  ])],
  ['scenario', 'Scenario', new Map([
    ['quiet', 'quiet'], ['energetic', 'energetic'],
    ['quiet-96', 'quiet-96'], ['energetic-126', 'energetic-126'],
    ['mellow-96', 'mellow-96'], ['driving-126', 'driving-126'],
  ])],
];
const DRAFT_PI_MAC = 'Pi 5 profile rendered on Mac GPU; not physical Pi hardware';
const BLOCKSCAPE_VARIANTS = new Map([
  ['base', 'base (Blockscape variant)'], ['tunnel', 'tunnel (Blockscape variant)'],
  ['stripes', 'stripes (Blockscape variant)'], ['stars', 'stars (Blockscape variant)'],
]);

export function validateCatalog(catalog) {
  if (!catalog || typeof catalog !== 'object' || Array.isArray(catalog) ||
      Object.keys(catalog).sort().join(',') !== 'clips,schema_version' ||
      catalog.schema_version !== 1 || !Array.isArray(catalog.clips)) throw new Error('Invalid catalog');
  const ids = new Set();
  for (const clip of catalog.clips) {
    if (!clip || typeof clip !== 'object' || Array.isArray(clip) ||
        Object.keys(clip).sort().join(',') !== [...FIELDS].sort().join(',') ||
        typeof clip.id !== 'string' || !ID.test(clip.id) || ids.has(clip.id) ||
        typeof clip.sha256 !== 'string' || !HASH.test(clip.sha256) ||
        !['title', 'family', 'platform', 'variant', 'scenario', 'capture']
          .every(key => typeof clip[key] === 'string' && clip[key].trim()) ||
        !(clip.source_revision === null || (typeof clip.source_revision === 'string' && REVISION.test(clip.source_revision))) ||
        !['library', 'draft'].includes(clip.collection) ||
        !Array.isArray(clip.notes) || !clip.notes.every(note => typeof note === 'string')) {
      throw new Error('Invalid clip');
    }
    ids.add(clip.id);
  }
  return catalog;
}

export function parseTime(value) {
  if (typeof value !== 'string') return null;
  const match = /^(?:(\d+):([0-5]\d)|(\d+))$/.exec(value.trim());
  if (!match) return null;
  const seconds = match[3] === undefined ? Number(match[1]) * 60 + Number(match[2]) : Number(match[3]);
  return Number.isSafeInteger(seconds) && seconds <= 2147483647 ? seconds : null;
}

// References deliberately contain no free-form metadata, paths, or URLs.
export function reference(clip, seconds) {
  if (!clip || typeof clip.id !== 'string' || !ID.test(clip.id) ||
      typeof clip.sha256 !== 'string' || !HASH.test(clip.sha256) ||
      !(clip.source_revision === null || (typeof clip.source_revision === 'string' && REVISION.test(clip.source_revision))) ||
      !Number.isSafeInteger(seconds) || seconds < 0 || seconds > 2147483647) throw new Error('Invalid reference');
  const context = [];
  for (const [field, label, allowed] of CONTEXT) {
    let value = allowed.get(clip[field]);
    if (value && field === 'platform' && value === 'Pi 5 profile' &&
        clip.collection === 'draft' && clip.capture === DRAFT_PI_MAC) {
      value = 'Pi 5 profile on Mac GPU (not physical Pi hardware)';
    }
    if (field === 'variant' && value === 'hallway' && clip.family === 'Pulse Tunnel') {
      value = 'hallway (Pulse Tunnel variant)';
    }
    if (field === 'variant' && clip.family === 'Blockscape') {
      value = BLOCKSCAPE_VARIANTS.get(clip.variant) || value;
    }
    if (value) context.push(`${label} (inferred): ${value}`);
  }
  if (clip.collection === 'draft') {
    context.push('Review-only draft; not human-reviewed. Do not publish.');
    context.push('Normal-speed human review and safety/flash assessment remain required.');
  }
  return [...context, `Clip ID: ${clip.id}`, `SHA-256: ${clip.sha256}`,
    `Declared source revision (unverified): ${clip.source_revision || 'unknown'}`,
    `Time: ${seconds} seconds`, 'Local reference; availability and review not verified.'].join('\n');
}
