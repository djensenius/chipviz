import { validateCatalog, parseTime, reference } from './lib.mjs';

const $ = id => document.getElementById(id);
const fields = [['family', 'Visualization'], ['platform', 'Platform'], ['variant', 'Variant'],
  ['scenario', 'Scenario'], ['collection', 'Collection']];
const filters = new Map();
let clips = [];
let selected = null;
let disposeDetail = null;
let clipboardWritePending = false;

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
    const context = [clip.title, clip.family, clip.platform, clip.variant, clip.scenario,
      clip.collection === 'draft' ? 'Review-only draft' : 'Local library', `Clip ID ${clip.id}`];
    button.setAttribute('aria-label', `${button.textContent} for ${context.join('; ')}`);
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
  if (disposeDetail) disposeDetail();
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
    if (disposeDetail !== discardVideo) return;
    discardVideo();
    disposeDetail = null;
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
  const fallback = node('textarea', null, 'fallback');
  fallback.readOnly = true;
  fallback.hidden = true;
  fallback.setAttribute('aria-label', 'Reference to copy');
  let video = null;
  let state = 'unopened';
  let copyVersion = 0;
  let copySeconds = null;
  let seekFeedbackVersion = null;
  function invalidateCopy() {
    copyVersion += 1;
    copySeconds = null;
    fallback.hidden = true;
  }
  function discardVideo() {
    invalidateCopy();
    const previous = video;
    video = null;
    state = 'unopened';
    if (previous) {
      previous.pause();
      previous.removeAttribute('src');
      previous.load();
    }
  }
  disposeDetail = discardVideo;
  function getTime() {
    const seconds = parseTime(input.value);
    if (seconds === null) {
      feedback.textContent = 'Enter a valid time in seconds or mm:ss.';
      input.focus();
    } else feedback.textContent = '';
    return seconds;
  }
  function hasDuration() {
    if (!video) feedback.textContent = 'Open the video and wait for metadata first.';
    else if (state === 'error') feedback.textContent = 'Video metadata or seek failed. Reopen the video and try again.';
    else if (state === 'loading' || !Number.isFinite(video.duration) || video.duration < 0) {
      feedback.textContent = 'Wait for valid video metadata, or reopen the video if loading failed.';
    } else return true;
    return false;
  }
  function inDuration(seconds) {
    if (!hasDuration()) return false;
    if (seconds > video.duration) {
      feedback.textContent = 'Time exceeds the video duration. Enter a time within the video.';
      return false;
    }
    return true;
  }
  const open = node('button', 'Open local video', 'primary');
  open.type = 'button';
  open.addEventListener('click', () => {
    if (disposeDetail !== discardVideo) return;
    invalidateCopy();
    const seconds = getTime();
    if (seconds === null) return;
    discardVideo();
    player.replaceChildren();
    const openedVideo = node('video', null);
    video = openedVideo;
    state = 'loading';
    openedVideo.controls = true;
    openedVideo.preload = 'metadata';
    openedVideo.addEventListener('error', () => {
      if (video !== openedVideo) return;
      invalidateCopy();
      state = 'error';
      feedback.textContent = 'Video metadata or seek failed. Reopen the video and try again.';
    });
    openedVideo.addEventListener('seeked', () => {
      if (video !== openedVideo || !['seeking', 'invalid'].includes(state) || openedVideo.seeking) return;
      if (!Number.isFinite(openedVideo.duration) || openedVideo.duration < 0 ||
          !Number.isFinite(openedVideo.currentTime) || openedVideo.currentTime < 0 ||
          openedVideo.currentTime > openedVideo.duration) {
        invalidateCopy();
        state = 'error';
        feedback.textContent = 'Video metadata or seek failed. Reopen the video and try again.';
        return;
      }
      state = 'ready';
      if (copySeconds !== null && copySeconds > openedVideo.duration) {
        invalidateCopy();
        feedback.textContent = 'Time exceeds the video duration. Enter a time within the video.';
      } else if (seekFeedbackVersion === copyVersion) feedback.textContent = '';
    });
    openedVideo.addEventListener('loadedmetadata', () => {
      if (video !== openedVideo || state !== 'loading') return;
      invalidateCopy();
      if (!Number.isFinite(openedVideo.duration) || openedVideo.duration < 0) {
        state = 'error';
        feedback.textContent = 'Video duration is unavailable. Reopen the video and try again.';
      } else if (seconds > openedVideo.duration) {
        state = 'invalid';
        seekFeedbackVersion = copyVersion;
        feedback.textContent = 'Time exceeds the video duration. Enter a time within the video.';
      } else if (openedVideo.currentTime === seconds && !openedVideo.seeking) {
        state = 'ready';
        feedback.textContent = '';
      } else {
        state = 'seeking';
        seekFeedbackVersion = copyVersion;
        try {
          openedVideo.currentTime = seconds;
          feedback.textContent = 'Waiting for the initial seek to finish.';
        } catch {
          state = 'error';
          feedback.textContent = 'Video metadata or seek failed. Reopen the video and try again.';
        }
      }
    }, { once: true });
    openedVideo.src = new URL(`media/${clip.id}`, location.href).href;
    player.append(openedVideo);
    openedVideo.focus();
    feedback.textContent = 'Loading video metadata.';
  });
  const current = node('button', 'Use current playback time', 'secondary');
  current.type = 'button';
  current.addEventListener('click', () => {
    invalidateCopy();
    if (!hasDuration()) return;
    if (state === 'invalid') {
      seekFeedbackVersion = copyVersion;
      feedback.textContent = 'Enter a valid time and reopen the video, or seek with the player.';
    } else if (state !== 'ready' || video.seeking) {
      seekFeedbackVersion = copyVersion;
      feedback.textContent = 'Wait for the video seek to finish before using playback time.';
    } else if (Number.isFinite(video.currentTime) && video.currentTime >= 0 && inDuration(video.currentTime)) {
      input.value = String(Math.floor(video.currentTime));
      feedback.textContent = '';
    } else if (!Number.isFinite(video.currentTime) || video.currentTime < 0) {
      feedback.textContent = 'Playback time is unavailable. Reopen the video and try again.';
    }
  });
  const copy = node('button', 'Copy reference', 'secondary');
  copy.type = 'button';
  input.addEventListener('input', () => {
    invalidateCopy();
    feedback.textContent = '';
  });
  copy.addEventListener('click', async () => {
    if (disposeDetail !== discardVideo) return;
    invalidateCopy();
    const seconds = getTime();
    if (seconds === null || !inDuration(seconds)) return;
    const text = reference(clip, seconds);
    const version = copyVersion;
    const inputValue = input.value;
    const copiedVideo = video;
    const isCurrent = () => disposeDetail === discardVideo && version === copyVersion &&
      input.value === inputValue && video === copiedVideo && state !== 'error' &&
      Number.isFinite(video.duration) && video.duration >= seconds;
    if (clipboardWritePending) {
      feedback.textContent = 'A clipboard write is still in progress. Wait and try copying again.';
      return;
    }
    copySeconds = seconds;
    try {
      if (!navigator.clipboard || !window.isSecureContext) throw new Error('Clipboard unavailable');
      clipboardWritePending = true;
      try {
        await navigator.clipboard.writeText(text);
      } finally {
        clipboardWritePending = false;
      }
      if (!isCurrent()) return;
      fallback.hidden = true;
      feedback.textContent = 'Reference copied.';
    } catch {
      if (!isCurrent()) return;
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
