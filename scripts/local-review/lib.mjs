const ID = /^clip-[0-9a-f]{20}$/;
const HASH = /^[0-9a-f]{64}$/;
const REVISION = /^[0-9a-f]{40}$/;
const FIELDS = ['id', 'sha256', 'title', 'family', 'platform', 'variant',
  'scenario', 'capture', 'source_revision', 'collection', 'notes'];

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
  return [`Clip ID: ${clip.id}`, `SHA-256: ${clip.sha256}`,
    `Declared source revision (unverified): ${clip.source_revision || 'unknown'}`,
    `Time: ${seconds} seconds`, 'Local reference; availability and review not verified.'].join('\n');
}
