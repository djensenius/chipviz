//! Static's beat-synced clock: which section is showing, how far into it,
//! and the bar-level changes (camera reframes and text flips). See the
//! module docs in `mod.rs` for the design; this file is the state machine.
//!
//! [`StaticState`] steps once per new frame, like Lines' state, and the
//! builder reads a [`Clock`] from it. Before the state has seen a frame the
//! clock follows the frame counter (`frame / 512`), which is also how a
//! state starts: it joins that schedule on its first frame. With procedural
//! input (a beat every 16 frames) sections then start on exactly the frames
//! the frame-timed schedule used.

use super::{Content, DISSOLVE_START, HOLD_START, SECTION_FRAMES, section_at};
use crate::control_frame::ControlFrame;

/// A bar starts when `beat_count` crosses a multiple of this.
pub(super) const BAR_BEATS: u16 = 4;
/// A forward `beat_count` step larger than this is a resync, not beats.
const MAX_BEAT_STEP: u16 = 4;
/// A forward `frame` step larger than this counts as one frame.
const MAX_FRAME_STEP: u16 = 8;
/// Frames without a beat before sections free-run on the frame count.
pub(super) const BEAT_STALL_FRAMES: u16 = 240;
/// Shortest gap between bar events: a bar line closer than this to the last
/// one (fast tempi, or a counter racing ahead) is ignored, not delayed.
pub(super) const MIN_BAR_FRAMES: u16 = 60;
/// Control frames per second assumed when converting `bpm` to frames.
const FRAME_RATE: f32 = 60.0;
/// Used when `bpm` is 0 (unknown).
const DEFAULT_BPM: f32 = 120.0;
/// A reframe eases over one beat, clamped to this range of frames.
pub(super) const MIN_EASE_FRAMES: f32 = 15.0;
const MAX_EASE_FRAMES: f32 = 30.0;
/// Text flips stagger each cell's switch over this many frames...
pub(super) const FLIP_SPREAD: u16 = 24;
/// ...then cycle it through random glyphs for this many before it lands.
pub(super) const FLIP_CYCLE: u16 = 12;
/// A flip must finish before the section's dissolve starts.
pub(super) const LAST_FLIP: u16 = DISSOLVE_START - FLIP_SPREAD - FLIP_CYCLE;

/// A camera framing applied on top of the section's move: yaw and pitch
/// (radians) are added, `zoom` scales the distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Framing {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

/// Head on, as the frame-timed schedule always is.
pub(super) const REST: Framing = Framing {
    yaw: 0.0,
    pitch: 0.0,
    zoom: 1.0,
};

/// The framings bars step through, in order. Every other one is head on so
/// text stays readable; the rest look from either side, from above or
/// pulled back. Close-ups stay at 0.9 or more so large text is not cropped.
pub(super) const FRAMINGS: [Framing; 6] = [
    REST,
    Framing {
        yaw: 0.42,
        pitch: 0.06,
        zoom: 1.12,
    },
    Framing {
        yaw: 0.0,
        pitch: 0.0,
        zoom: 0.9,
    },
    Framing {
        yaw: -0.42,
        pitch: -0.05,
        zoom: 1.12,
    },
    REST,
    Framing {
        yaw: 0.12,
        pitch: 0.32,
        zoom: 1.3,
    },
];

/// One bar's ease in frames at `bpm`: a beat, clamped.
fn ease_frames(bpm: u8) -> f32 {
    let bpm = if bpm == 0 {
        DEFAULT_BPM
    } else {
        f32::from(bpm)
    };
    (60.0 * FRAME_RATE / bpm).clamp(MIN_EASE_FRAMES, MAX_EASE_FRAMES)
}

/// What the builder needs from the state for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Clock {
    /// Section count; its value picks content, style, move and background.
    pub index: u16,
    /// Frames into the section.
    pub local: u16,
    /// Text flips so far in this section.
    pub flips: u16,
    /// `local` when the last flip started.
    pub flip_at: u16,
    pub framing: Framing,
}

impl Clock {
    /// The frame-timed schedule: a section every 512 frames, head on.
    pub(super) fn frame_timed(frame: ControlFrame) -> Self {
        Self {
            index: frame.frame / SECTION_FRAMES,
            local: frame.frame % SECTION_FRAMES,
            flips: 0,
            flip_at: 0,
            framing: REST,
        }
    }
}

/// The fields [`StaticState::observe`] compares between frames.
#[derive(Clone, Copy, Debug)]
struct Seen {
    frame: u16,
    beat_count: u16,
}

/// Static's section and bar state, kept between frames (see the module docs).
#[derive(Clone, Debug)]
pub struct StaticState {
    last: Option<Seen>,
    index: u16,
    local: u16,
    /// Frames since `beat_count` last advanced.
    stalled: u16,
    /// Frames since the last bar event, saturating.
    since_bar: u16,
    /// Bar events so far; picks the framing.
    bars: u32,
    /// Framing when the last bar event started its ease.
    from: Framing,
    /// That ease's length in frames.
    ease: f32,
    flips: u16,
    flip_at: u16,
}

impl Default for StaticState {
    fn default() -> Self {
        Self {
            last: None,
            index: 0,
            local: 0,
            stalled: 0,
            // The first bar is never held back.
            since_bar: u16::MAX,
            bars: 0,
            from: REST,
            ease: MIN_EASE_FRAMES,
            flips: 0,
            flip_at: 0,
        }
    }
}

impl StaticState {
    /// Steps on one new frame. A frame whose counter repeats the last one is
    /// ignored, like Lines' state.
    pub fn observe(&mut self, frame: &ControlFrame) {
        let seen = Seen {
            frame: frame.frame,
            beat_count: frame.beat_count,
        };
        let Some(last) = self.last else {
            // Join the frame-timed schedule.
            let clock = Clock::frame_timed(*frame);
            (self.index, self.local) = (clock.index, clock.local);
            self.last = Some(seen);
            return;
        };
        let step = frame.frame.wrapping_sub(last.frame);
        if step == 0 {
            return;
        }
        self.last = Some(seen);
        let frames = if step <= MAX_FRAME_STEP { step } else { 1 };
        self.local = self.local.saturating_add(frames);
        self.since_bar = self.since_bar.saturating_add(frames);
        let beats = frame.beat_count.wrapping_sub(last.beat_count);
        let advanced = (1..=MAX_BEAT_STEP).contains(&beats);
        self.stalled = if advanced {
            0
        } else {
            self.stalled.saturating_add(frames)
        };
        let bar = advanced && frame.beat_count % BAR_BEATS < beats;
        // A section runs its whole 512-frame timeline, then the next one
        // starts on a bar line (or at once when the beat has stalled).
        if self.local >= SECTION_FRAMES && (bar || self.stalled >= BEAT_STALL_FRAMES) {
            self.index = self.index.wrapping_add(1);
            self.local = 0;
            self.flips = 0;
            self.flip_at = 0;
        }
        if bar && self.since_bar >= MIN_BAR_FRAMES {
            self.on_bar(frame.bpm);
        }
    }

    /// A bar line: reframe the camera, and in a text section's hold flip the
    /// phrase to another language.
    fn on_bar(&mut self, bpm: u8) {
        self.from = self.framing();
        self.bars = self.bars.wrapping_add(1);
        self.since_bar = 0;
        self.ease = ease_frames(bpm);
        let content = section_at(self.clock_parts()).content;
        if matches!(content, Content::Text(_)) && (HOLD_START..=LAST_FLIP).contains(&self.local) {
            self.flips = self.flips.saturating_add(1);
            self.flip_at = self.local;
        }
    }

    fn clock_parts(&self) -> Clock {
        Clock {
            index: self.index,
            local: self.local,
            flips: self.flips,
            flip_at: self.flip_at,
            framing: REST,
        }
    }

    /// The framing now: eased (smoothstep) from where the camera was at the
    /// last bar to that bar's framing.
    fn framing(&self) -> Framing {
        let target = FRAMINGS[self.bars as usize % FRAMINGS.len()];
        let t = (f32::from(self.since_bar) / self.ease).clamp(0.0, 1.0);
        if t >= 1.0 {
            return target;
        }
        let eased = t * t * (3.0 - 2.0 * t);
        let lerp = |a: f32, b: f32| a + (b - a) * eased;
        Framing {
            yaw: lerp(self.from.yaw, target.yaw),
            pitch: lerp(self.from.pitch, target.pitch),
            zoom: lerp(self.from.zoom, target.zoom),
        }
    }

    /// The clock for `frame`: the frame-timed one until the state has seen
    /// a frame.
    pub(super) fn clock(&self, frame: ControlFrame) -> Clock {
        if self.last.is_none() {
            return Clock::frame_timed(frame);
        }
        Clock {
            framing: self.framing(),
            ..self.clock_parts()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::HOLD_START;
    use super::*;

    /// `live_bridge`'s timing (see Lines' tests): a beat every 30 frames.
    fn live(index: u64) -> ControlFrame {
        ControlFrame {
            beat_count: (index / 30) as u16,
            ..ControlFrame::procedural(index)
        }
    }

    /// Steps a fresh state through `frames`, returning the clock after each.
    fn run(frames: impl IntoIterator<Item = ControlFrame>) -> Vec<Clock> {
        let mut state = StaticState::default();
        frames
            .into_iter()
            .map(|frame| {
                state.observe(&frame);
                state.clock(frame)
            })
            .collect()
    }

    /// Frames (indices into `clocks`) where a section starts, and where a
    /// bar reframes the camera.
    fn events(clocks: &[Clock], frames: &[ControlFrame]) -> (Vec<usize>, Vec<usize>) {
        let mut state = StaticState::default();
        let mut sections = Vec::new();
        let mut bars = Vec::new();
        for (index, frame) in frames.iter().enumerate() {
            let before = (state.index, state.bars);
            state.observe(frame);
            assert_eq!(state.clock(*frame), clocks[index]);
            if index > 0 && state.index != before.0 {
                sections.push(index);
            }
            if state.bars != before.1 {
                bars.push(index);
            }
        }
        (sections, bars)
    }

    #[test]
    fn unobserved_state_is_the_frame_timed_schedule() {
        let state = StaticState::default();
        for index in [0, 1, 511, 512, 4000, 65_535] {
            let frame = ControlFrame::procedural(index);
            assert_eq!(state.clock(frame), Clock::frame_timed(frame));
        }
        // A state joins that schedule on its first frame, wherever it is.
        let frame = ControlFrame::procedural(4908);
        let clocks = run([frame]);
        assert_eq!(clocks[0], Clock::frame_timed(frame));
    }

    #[test]
    fn procedural_sections_land_on_the_frame_timed_frames() {
        // A beat every 16 frames: a bar every 64, so sections start on bar
        // lines at exactly 512-frame steps, as before.
        let frames: Vec<_> = (0..8192).map(ControlFrame::procedural).collect();
        let clocks = run(frames.iter().copied());
        for (frame, clock) in frames.iter().zip(&clocks) {
            let timed = Clock::frame_timed(*frame);
            assert_eq!((clock.index, clock.local), (timed.index, timed.local));
        }
        let (sections, bars) = events(&clocks, &frames);
        assert_eq!(sections, (1..16).map(|n| n * 512).collect::<Vec<_>>());
        assert_eq!(bars, (1..128).map(|n| n * 64).collect::<Vec<_>>());
        // Text sections (0, 4, 8, ...) flip at the bars inside the hold:
        // local 256 and 320 (384 is too close to the dissolve).
        for section in [0usize, 4, 8] {
            let at = |local: usize| clocks[section * 512 + local];
            assert_eq!((at(255).flips, at(256).flips), (0, 1), "{section}");
            assert_eq!((at(319).flips, at(320).flips), (1, 2), "{section}");
            assert_eq!((at(320).flip_at, at(511).flips), (320, 2), "{section}");
        }
        // Other content never flips.
        for (index, clock) in clocks.iter().enumerate() {
            if index / 512 % 4 != 0 {
                assert_eq!(clock.flips, 0, "frame {index}");
            }
        }
    }

    #[test]
    fn live_sections_start_on_the_first_bar_after_their_timeline() {
        // A bar every 120 frames: the first bar line at or past local 512
        // is 600 frames into the section.
        let frames: Vec<_> = (0..3000).map(live).collect();
        let clocks = run(frames.iter().copied());
        let (sections, bars) = events(&clocks, &frames);
        assert_eq!(sections, [600, 1200, 1800, 2400]);
        assert_eq!(bars, (1..25).map(|n| n * 120).collect::<Vec<_>>());
        for &start in &sections {
            assert_eq!(frames[start].beat_count % BAR_BEATS, 0);
            assert_eq!(clocks[start].local, 0);
            assert_eq!(clocks[start - 1].local, 599);
        }
        // Section 0 is text: its one bar in the hold (local 360) flips it.
        assert_eq!((clocks[359].flips, clocks[360].flips), (0, 1));
        // Section 4 (frames 2400..) is text again; its hold bar is 2760.
        assert_eq!(clocks[2400].index, 4);
        assert_eq!((clocks[2759].flips, clocks[2760].flips), (0, 1));
    }

    #[test]
    fn racing_beats_keep_bars_a_second_apart_and_sections_whole() {
        // Worst case: the counter ticks every frame at 255 bpm, so a bar
        // line passes every 4 frames.
        let frames: Vec<_> = (0..4000)
            .map(|index| ControlFrame {
                bpm: 255,
                beat_count: index as u16,
                ..ControlFrame::procedural(index)
            })
            .collect();
        let clocks = run(frames.iter().copied());
        let (sections, bars) = events(&clocks, &frames);
        assert!(bars.len() > 50);
        assert!(
            bars.windows(2)
                .all(|pair| pair[1] - pair[0] >= usize::from(MIN_BAR_FRAMES))
        );
        assert!(sections.len() >= 6);
        let mut starts = vec![0];
        starts.extend(&sections);
        assert!(starts.windows(2).all(|pair| pair[1] - pair[0] >= 512));
    }

    #[test]
    fn stalled_beats_free_run_every_512_frames_without_reframing() {
        let frames: Vec<_> = (100..3000)
            .map(|index| ControlFrame {
                beat_count: 9,
                ..ControlFrame::procedural(index)
            })
            .collect();
        let clocks = run(frames.iter().copied());
        let (sections, bars) = events(&clocks, &frames);
        // Joined at local 100: the first section ends at 512 (index 412).
        assert_eq!(sections, [412, 924, 1436, 1948, 2460]);
        assert!(bars.is_empty());
        assert!(clocks.iter().all(|clock| clock.framing == REST));
    }

    #[test]
    fn counter_jumps_resync_and_repeated_frames_do_not_step() {
        let mut state = StaticState::default();
        for index in 0..200 {
            state.observe(&ControlFrame::procedural(index));
        }
        let clock = state.clock(ControlFrame::procedural(199));
        // A repeated frame counter is not a new frame.
        for _ in 0..30 {
            state.observe(&ControlFrame::procedural(199));
        }
        assert_eq!(state.clock(ControlFrame::procedural(199)), clock);
        // A backward or large beat jump (a restart or handover) is no bar,
        // and a large frame jump counts as one frame.
        let jumps = [(4000, 2), (4001, 900)];
        for (frame, beat_count) in jumps {
            let bars = state.bars;
            let local = state.local;
            state.observe(&ControlFrame {
                beat_count,
                ..ControlFrame::procedural(frame)
            });
            assert_eq!(state.bars, bars);
            assert_eq!(state.local, local + 1);
        }
    }

    #[test]
    fn reframes_ease_smoothly_and_flips_finish_before_the_dissolve() {
        // Every tempo from racing to slow: the framing moves at most the
        // smoothstep's peak slope over the shortest ease per frame.
        let biggest = FRAMINGS
            .iter()
            .flat_map(|a| FRAMINGS.iter().map(move |b| (a, b)))
            .map(|(a, b)| [b.yaw - a.yaw, b.pitch - a.pitch, b.zoom - a.zoom].map(f32::abs))
            .fold([0.0f32; 3], |most, delta| {
                std::array::from_fn(|axis| most[axis].max(delta[axis]))
            });
        for frames_per_beat in [1, 4, 16, 30, 60] {
            let frames: Vec<_> = (0..3000)
                .map(|index| ControlFrame {
                    bpm: (3600 / frames_per_beat).min(255) as u8,
                    beat_count: (index / frames_per_beat) as u16,
                    ..ControlFrame::procedural(index)
                })
                .collect();
            let clocks = run(frames.iter().copied());
            for pair in clocks.windows(2) {
                let [a, b] = [pair[0].framing, pair[1].framing];
                let step = [b.yaw - a.yaw, b.pitch - a.pitch, b.zoom - a.zoom];
                for axis in 0..3 {
                    let bound = biggest[axis] * 1.5 / MIN_EASE_FRAMES + 1e-6;
                    assert!(step[axis].abs() <= bound, "{frames_per_beat}: {step:?}");
                }
                let flip = pair[1];
                if flip.flips > 0 {
                    assert!((HOLD_START..=LAST_FLIP).contains(&flip.flip_at));
                }
            }
        }
        assert_eq!(ease_frames(0), 30.0);
        assert_eq!(ease_frames(255), MIN_EASE_FRAMES);
    }

    #[test]
    fn states_are_deterministic() {
        let frames: Vec<_> = (0..3000).map(live).collect();
        assert_eq!(run(frames.iter().copied()), run(frames.iter().copied()));
    }
}
