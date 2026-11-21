import { validateCatalog, parseTime, reference } from './lib.mjs';

const $ = id => document.getElementById(id);
const fields = [['family', 'Visualization'], ['platform', 'Platform'], ['variant', 'Variant'],
  ['scenario', 'Scenario'], ['collection', 'Collection']];
const filters = new Map();
let clips = [];
let selected = null;

function node(tag, text, className) {
  const element = document.createElement(tag);
  if (text !== null) element.textContent = text;
  if (className) element.className = className;
  return element;
}

function render() {
  const term = $('search').value.trim().toLocaleLowerCase();
  const visible = clips.filter(clip =>
    [...filters].every(([key, select]) => !select.value || clip[key] === select.value) &&
    (!term || [clip.title, clip.family, clip.platform, clip.variant, clip.scenario, clip.capture,
      clip.id, clip.source_revision || ''].some(value => value.toLocaleLowerCase().includes(term))));
  $('count').textContent = `${visible.length} ${visible.length === 1 ? 'capture' : 'captures'}`;
  $('results').replaceChildren();
  for (const clip of visible) {
    const card = node('article', null, 'card');
    const button = node('button', selected === clip ? 'Selected' : 'View details', 'card-button');
    button.type = 'button';
    button.setAttribute('aria-label', `View details for ${clip.title}`);
    button.addEventListener('click', () => selectClip(clip));
    card.append(node('p', `${clip.family} / ${clip.platform}`, 'card-label'),
      node('h3', clip.title), node('p', `${clip.scenario} · ${clip.variant}`, 'muted'),
      node('p', `${clip.collection === 'draft' ? 'Review-only draft · ' : ''}${clip.capture}`, 'provenance'), button);
    $('results').append(card);
  }
  $('message').textContent = clips.length === 0 ? 'No local captures found. Refresh the page to scan again.' :
    visible.length === 0 ? 'No captures match these filters.' : '';
  $('message').hidden = !$('message').textContent;
}

function selectClip(clip) {
  selected = clip;
  const detail = $('detail');
  detail.hidden = false;
  detail.replaceChildren();
  const header = node('div', null, 'detail-head');
  const heading = node('h2', clip.title);
  heading.tabIndex = -1;
  const close = node('button', 'Close details', 'text-button');
  close.type = 'button';
  close.addEventListener('click', () => {
    selected = null;
    detail.hidden = true;
    detail.replaceChildren();
    render();
    $('search').focus();
  });
  header.append(heading, close);
  detail.append(node('p', 'SELECTED CAPTURE', 'eyebrow'), header);
  const meta = node('dl', null, 'metadata');
  for (const [label, value] of [
    ['Collection', clip.collection === 'draft' ? 'Review-only draft' : 'Local library'],
    ['Visualization (inferred)', clip.family], ['Platform (inferred)', clip.platform],
    ['Scenario (inferred)', clip.scenario], ['Variant (inferred)', clip.variant],
    ['Capture (inferred)', clip.capture], ['Declared source revision (unverified)', clip.source_revision || 'Unknown'],
    ['Clip ID', clip.id], ['SHA-256', clip.sha256],
  ]) {
    const item = node('div', null);
    item.append(node('dt', label), node('dd', value));
    meta.append(item);
  }
  detail.append(meta);
  const list = node('ul', null, 'notes');
  for (const note of clip.notes) list.append(node('li', note));
  detail.append(list);

  const timeLabel = node('label', 'Time (mm:ss or seconds)');
  const input = node('input', null);
  input.type = 'text';
  input.inputMode = 'numeric';
  input.value = '0';
  input.maxLength = 12;
  timeLabel.append(input);
  const feedback = node('p', '', 'feedback');
  feedback.setAttribute('role', 'status');
  const player = node('div', null, 'player');
  const controls = node('div', null, 'actions');
  let video = null;
  function getTime() {
    const seconds = parseTime(input.value);
    if (seconds === null) {
      feedback.textContent = 'Enter a valid time in seconds or mm:ss.';
      input.focus();
    } else feedback.textContent = '';
    return seconds;
  }
  const open = node('button', 'Open local video', 'primary');
  open.type = 'button';
  open.addEventListener('click', () => {
    const seconds = getTime();
    if (seconds === null) return;
    player.replaceChildren();
    video = node('video', null);
    video.controls = true;
    video.preload = 'metadata';
    video.src = new URL(`media/${clip.id}`, location.href).href;
    video.addEventListener('loadedmetadata', () => { video.currentTime = seconds; }, { once: true });
    player.append(video);
    video.focus();
  });
  const current = node('button', 'Use current playback time', 'secondary');
  current.type = 'button';
  current.addEventListener('click', () => {
    if (video && Number.isFinite(video.currentTime)) input.value = String(Math.floor(video.currentTime));
    else feedback.textContent = 'Open a video first.';
  });
  const copy = node('button', 'Copy reference', 'secondary');
  copy.type = 'button';
  const fallback = node('textarea', null, 'fallback');
  fallback.readOnly = true;
  fallback.hidden = true;
  fallback.setAttribute('aria-label', 'Reference to copy');
  copy.addEventListener('click', async () => {
    const seconds = getTime();
    if (seconds === null) return;
    const text = reference(clip, seconds);
    try {
      if (!navigator.clipboard || !window.isSecureContext) throw new Error('Clipboard unavailable');
      await navigator.clipboard.writeText(text);
      fallback.hidden = true;
      feedback.textContent = 'Reference copied.';
    } catch {
      fallback.value = text;
      fallback.hidden = false;
      feedback.textContent = 'Clipboard unavailable. Select and copy the reference below.';
      fallback.focus();
      fallback.select();
    }
  });
  controls.append(open, current, copy);
  detail.append(timeLabel, controls, feedback, fallback, player);
  render();
  heading.focus();
}

async function init() {
  try {
    const response = await fetch(new URL('catalog.json', location.href), { referrerPolicy: 'no-referrer' });
    if (!response.ok) throw new Error('Catalog request failed');
    clips = validateCatalog(await response.json()).clips;
    for (const [key, label] of fields) {
      const wrapper = node('div', null, 'filter');
      const caption = node('label', label);
      const select = node('select', null);
      select.id = `filter-${key}`;
      caption.htmlFor = select.id;
      select.append(node('option', 'All'));
      select.firstChild.value = '';
      for (const value of [...new Set(clips.map(clip => clip[key]))].sort((a, b) => a.localeCompare(b))) {
        const option = node('option', value);
        option.value = value;
        select.append(option);
      }
      select.addEventListener('change', render);
      filters.set(key, select);
      wrapper.append(caption, select);
      $('filter-fields').append(wrapper);
    }
    $('search').addEventListener('input', render);
    $('reset').addEventListener('click', () => {
      $('search').value = '';
      for (const select of filters.values()) select.value = '';
      render();
    });
    render();
  } catch {
    $('count').textContent = 'Catalog unavailable';
    $('message').textContent = 'The catalog could not be loaded or is invalid. Refresh to try again.';
    $('message').hidden = false;
  }
}

init();
