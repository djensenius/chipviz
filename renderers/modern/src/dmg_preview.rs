//! Isolated preview clock. One DMG frame is 70,224 cycles at 4,194,304 Hz.
use std::time::{Duration, Instant};

use crate::scenes::dmg_tile_scroll::{Display, DmgTileScroll};
use crate::scenes::dmg_tile_scroll_render::DmgTileRenderer;

const CYCLES_PER_SECOND: u128 = 4_194_304;
const FRAME_CYCLES_NANOS: u128 = 70_224 * 1_000_000_000;
// Less than a 60 Hz refresh (16.667 ms), but prevents adjacent bucket
// boundaries from producing near-instant successive submissions.
const MIN_TRANSITION_SPACING: Duration = Duration::from_millis(16);

/// Keep failed surface acquisitions from sampling the clock or stepping.
pub(crate) fn acquired_dmg_frame<T, E>(
    output: Result<T, E>,
    preview: &mut DmgPreview,
    now: impl FnOnce() -> Instant,
) -> Result<(T, Display), E> {
    let output = output?;
    Ok((output, preview.acquired(now())))
}

#[derive(Default)]
pub(crate) struct DmgPreview {
    model: DmgTileScroll,
    epoch: Option<Instant>,
    last_interval: u128,
    last_submission: Option<Instant>,
    started: bool,
    pending_transition: bool,
}

impl DmgPreview {
    /// Called only after a nonzero surface is acquired. The first acquisition
    /// runs ApplyDisplay/NativeStep; its clock starts after submission/present.
    /// Later ticks follow the phase at acquisition. Missed intervals are
    /// dropped, not replayed; submission is separately paced after encoding.
    pub(crate) fn acquired(&mut self, now: Instant) -> Display {
        if !self.started {
            self.started = true;
            self.pending_transition = true;
            self.model.tick();
            return self.model.shown();
        }
        if let Some(epoch) = self.epoch {
            let interval = now.saturating_duration_since(epoch).as_nanos() * CYCLES_PER_SECOND
                / FRAME_CYCLES_NANOS;
            if !self.pending_transition && interval > self.last_interval {
                self.last_interval = interval;
                self.pending_transition = true;
                self.model.tick();
            }
        }
        self.model.shown()
    }

    /// Called after encoding and before submit. For a changed frame, wait
    /// only the remaining part of 16 ms since the last changed submission. A
    /// monotonic clock bounds the requested sleep to 16 ms; it never spins.
    fn pace_submission(&self, now: Instant, wait: impl FnOnce(Duration)) {
        if self.pending_transition
            && let Some(last) = self.last_submission
        {
            let remaining =
                MIN_TRANSITION_SPACING.saturating_sub(now.saturating_duration_since(last));
            if !remaining.is_zero() {
                wait(remaining);
            }
        }
    }

    /// Record the changed submission immediately before queue.submit.
    pub(crate) fn submitted(&mut self, now: Instant) {
        if self.pending_transition {
            self.last_submission = Some(now);
        }
    }

    /// The initial epoch is sampled after present returns; neither timestamp
    /// establishes when a compositor or display actually shows the frame.
    pub(crate) fn presented(&mut self, now: Instant) {
        if self.pending_transition {
            if self.epoch.is_none() {
                self.epoch = Some(now);
            }
            self.pending_transition = false;
        }
    }
}

/// Finalization can be slow, so it must finish before sampling the pacing
/// clock or the timestamp immediately preceding submission.
fn finish_and_submit<C>(
    preview: &mut DmgPreview,
    finish: impl FnOnce() -> C,
    mut now: impl FnMut() -> Instant,
    wait: impl FnOnce(Duration),
    submit: impl FnOnce(C),
) {
    let commands = finish();
    preview.pace_submission(now(), wait);
    preview.submitted(now());
    submit(commands);
}

/// Surface and offscreen callers use the same acquisition, encode and submit
/// orchestration. The target's present callback may be a no-op offscreen.
pub(crate) struct DmgTarget<A, V, P> {
    pub acquire: A,
    pub view: V,
    pub present: P,
}

pub(crate) struct DmgPresenter {
    renderer: DmgTileRenderer,
    preview: DmgPreview,
}

impl DmgPresenter {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            renderer: DmgTileRenderer::new(device, format),
            preview: DmgPreview::default(),
        }
    }

    pub(crate) fn render<T, E>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: (u32, u32),
        target: DmgTarget<
            impl FnOnce() -> Result<T, E>,
            impl FnOnce(&T) -> wgpu::TextureView,
            impl FnOnce(T),
        >,
        mut now: impl FnMut() -> Instant,
        wait: impl FnOnce(Duration),
    ) -> Result<bool, E> {
        if size.0 == 0 || size.1 == 0 {
            return Ok(false);
        }
        let (output, display) =
            acquired_dmg_frame((target.acquire)(), &mut self.preview, &mut now)?;
        let view = (target.view)(&output);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("DMG preview encoder"),
        });
        assert!(
            self.renderer
                .encode(queue, &mut encoder, &view, size, display)
        );
        finish_and_submit(
            &mut self.preview,
            || encoder.finish(),
            &mut now,
            wait,
            |commands| {
                queue.submit(std::iter::once(commands));
            },
        );
        (target.present)(output);
        self.preview.presented(now());
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::dmg_tile_scroll_render::shade_pixels;

    fn draw(preview: &mut DmgPreview, at: Instant) -> Display {
        let (display, submitted, wait) = simulate_submission(preview, at, at);
        assert_eq!(wait, Duration::ZERO);
        assert_eq!(submitted, at);
        display
    }

    fn simulate_submission(
        preview: &mut DmgPreview,
        acquired: Instant,
        encoded: Instant,
    ) -> (Display, Instant, Duration) {
        let display = preview.acquired(acquired);
        let mut wait = Duration::ZERO;
        preview.pace_submission(encoded, |duration| wait = duration);
        let submitted = encoded + wait;
        preview.submitted(submitted);
        preview.presented(submitted);
        (display, submitted, wait)
    }

    #[test]
    fn startup_boundaries_and_pixel_pipeline() {
        let start = Instant::now();
        let mut preview = DmgPreview::default();
        let initial = draw(&mut preview, start);
        assert_eq!(
            &shade_pixels(initial)[8..16],
            &[0, 255, 255, 255, 255, 255, 255, 0]
        );
        assert_eq!(preview.model.counter(), 1);
        assert_eq!(
            draw(&mut preview, start + Duration::from_nanos(16_742_706)),
            initial
        );
        let next = draw(&mut preview, start + Duration::from_nanos(16_742_707));
        assert_eq!(preview.model.counter(), 2);
        assert_eq!(
            &shade_pixels(next)[0..8],
            &[255, 255, 255, 0, 0, 255, 255, 255]
        );
        assert_eq!(
            draw(&mut preview, start + Duration::from_nanos(16_742_707)),
            next
        );
    }

    #[test]
    fn failed_acquisition_resize_and_stall_drop_intervals_without_burst() {
        let start = Instant::now();
        let mut preview = DmgPreview::default();
        // A failed or zero-size acquisition never calls acquired, even at startup.
        assert_eq!(preview.model.counter(), 0);
        draw(&mut preview, start);
        // A failed acquisition or resize at 20 ms does not tick or reset.
        let resumed = start + Duration::from_secs(10);
        draw(&mut preview, resumed);
        assert_eq!(preview.model.counter(), 2);
        draw(&mut preview, resumed);
        assert_eq!(preview.model.counter(), 2);
        // A backwards timestamp cannot rewind the interval.
        draw(&mut preview, start);
        assert_eq!(preview.model.counter(), 2);
        draw(&mut preview, resumed + Duration::from_millis(20));
        assert_eq!(preview.model.counter(), 3);
    }

    #[test]
    fn fractional_period_does_not_accumulate_rounding_drift() {
        let start = Instant::now();
        let mut preview = DmgPreview::default();
        draw(&mut preview, start);
        for interval in 1..=120_u128 {
            let boundary = (interval * FRAME_CYCLES_NANOS).div_ceil(CYCLES_PER_SECOND) as u64;
            draw(&mut preview, start + Duration::from_nanos(boundary - 1));
            assert_eq!(preview.model.counter(), interval as u8);
            draw(&mut preview, start + Duration::from_nanos(boundary));
            assert_eq!(preview.model.counter(), interval as u8 + 1);
        }
    }

    #[test]
    fn simulated_redraw_rates_follow_same_clock_and_bound_stall_recovery() {
        fn simulate(hz: u64, stall: bool) -> Vec<(u32, Display)> {
            let start = Instant::now();
            let mut preview = DmgPreview::default();
            let mut ticks = 0;
            let mut checkpoints = Vec::new();
            for redraw in 0..=60 * hz {
                if stall && redraw > 2 * hz && redraw < 3 * hz {
                    continue;
                }
                let now = start + Duration::from_nanos(redraw * 1_000_000_000 / hz);
                let before = preview.model.counter();
                let display = draw(&mut preview, now);
                let advanced = preview.model.counter().wrapping_sub(before);
                assert!(advanced <= 1, "{hz} Hz, redraw {redraw}");
                ticks += u32::from(advanced);
                if stall && redraw == 3 * hz {
                    assert_eq!(advanced, 1, "resume advances exactly once");
                    assert_eq!(draw(&mut preview, now), display);
                    assert_eq!(preview.model.counter(), before.wrapping_add(1));
                }
                if redraw % hz == 0 && [0, 1, 2, 3, 10, 60].contains(&(redraw / hz)) {
                    checkpoints.push((ticks, display));
                }
            }
            checkpoints
        }

        for stall in [false, true] {
            let reference = simulate(60, stall);
            let expected = if stall {
                [1, 60, 120, 121, 539, 3525]
            } else {
                [1, 60, 120, 180, 598, 3584]
            };
            assert_eq!(
                reference
                    .iter()
                    .map(|(ticks, _)| *ticks)
                    .collect::<Vec<_>>(),
                expected
            );
            for hz in [120, 240] {
                assert_eq!(simulate(hz, stall), reference, "{hz} Hz, stall={stall}");
            }
        }
    }

    #[test]
    fn slow_first_submission_holds_initial_frame_for_a_native_period_after_present() {
        let start = Instant::now();
        let mut preview = DmgPreview::default();
        let first = preview.acquired(start);
        assert_eq!(preview.model.counter(), 1);
        let submitted = start + Duration::from_millis(92);
        let presented = start + Duration::from_millis(100);
        assert_eq!(preview.acquired(submitted), first);
        preview.submitted(submitted);
        assert_eq!(preview.acquired(presented), first);
        preview.presented(presented);
        assert_eq!(
            draw(&mut preview, presented + Duration::from_nanos(16_742_706)),
            first
        );
        assert_eq!(preview.model.counter(), 1);
        let second = draw(&mut preview, presented + Duration::from_nanos(16_742_707));
        assert_eq!(preview.model.counter(), 2);
        assert_ne!(second, first);
    }

    #[test]
    fn failed_acquisition_preserves_submission_and_startup_clocks() {
        let start = Instant::now();
        let mut preview = DmgPreview::default();
        assert_eq!(
            acquired_dmg_frame::<(), _>(Err("lost"), &mut preview, || panic!("sampled failure")),
            Err("lost")
        );
        assert!(preview.epoch.is_none());
        assert!(preview.last_submission.is_none());
        let (_, initial) = acquired_dmg_frame::<_, ()>(Ok(()), &mut preview, || start).unwrap();
        let submitted = start + Duration::from_millis(1);
        let presented = start + Duration::from_millis(9);
        preview.submitted(submitted);
        preview.presented(presented);
        assert_eq!(
            acquired_dmg_frame::<(), _>(Err("timeout"), &mut preview, || panic!("sampled failure")),
            Err("timeout")
        );
        assert_eq!(preview.epoch, Some(presented));
        assert_eq!(preview.last_submission, Some(submitted));
        assert_eq!(
            preview.acquired(presented + Duration::from_nanos(16_742_706)),
            initial
        );
        assert_eq!(preview.model.counter(), 1);
        preview.acquired(presented + Duration::from_nanos(16_742_707));
        assert_eq!(preview.model.counter(), 2);
    }

    #[test]
    fn late_or_jittery_acquisitions_cannot_cross_adjacent_buckets_in_under_16ms() {
        let start = Instant::now();
        let mut preview = DmgPreview::default();
        draw(&mut preview, start);
        // Resume just before the second native boundary.
        let late = start + Duration::from_nanos(33_485_412);
        let (displayed, first_submission, wait) = simulate_submission(&mut preview, late, late);
        assert_eq!(wait, Duration::ZERO);
        assert_eq!(preview.model.counter(), 2);
        let (next, second_submission, wait) = simulate_submission(
            &mut preview,
            late + Duration::from_nanos(1),
            late + Duration::from_nanos(1),
        );
        assert_ne!(next, displayed);
        assert_eq!(wait, Duration::from_nanos(15_999_999));
        assert_eq!(
            second_submission.duration_since(first_submission),
            MIN_TRANSITION_SPACING
        );
        assert_eq!(preview.model.counter(), 3);

        // A slow encode crosses the deadline and does not add another wait.
        let acquired = late + Duration::from_millis(40);
        let (_, submitted, wait) =
            simulate_submission(&mut preview, acquired, acquired + Duration::from_millis(20));
        assert_eq!(wait, Duration::ZERO);
        assert_eq!(submitted, acquired + Duration::from_millis(20));
        assert_eq!(preview.model.counter(), 4);
        let (_, next_submission, wait) = simulate_submission(
            &mut preview,
            submitted + Duration::from_millis(15),
            submitted + Duration::from_millis(15),
        );
        assert_eq!(wait, Duration::from_millis(1));
        assert_eq!(
            next_submission.duration_since(submitted),
            MIN_TRANSITION_SPACING
        );
        assert_eq!(preview.model.counter(), 5);
    }

    #[test]
    fn eight_ms_present_latency_does_not_extend_submission_spacing() {
        let start = Instant::now();
        let mut preview = DmgPreview::default();
        preview.acquired(start);
        preview.submitted(start + Duration::from_millis(1));
        preview.presented(start + Duration::from_millis(9));
        preview.acquired(start + Duration::from_millis(34));
        preview.submitted(start + Duration::from_millis(35));
        preview.presented(start + Duration::from_millis(43));
        let (_, submitted, wait) = simulate_submission(
            &mut preview,
            start + Duration::from_millis(51),
            start + Duration::from_millis(52),
        );
        assert_eq!(preview.model.counter(), 3);
        assert_eq!(wait, Duration::ZERO);
        assert_eq!(
            submitted.duration_since(start + Duration::from_millis(35)),
            Duration::from_millis(17)
        );
    }

    #[test]
    fn delayed_finalization_is_included_before_pacing_and_submission_timestamp() {
        use std::cell::Cell;

        let start = Instant::now();
        let mut preview = DmgPreview::default();
        draw(&mut preview, start);
        let late = start + Duration::from_nanos(33_485_412);
        preview.acquired(late);
        let clock = Cell::new(late);
        let finalized = Cell::new(false);
        let first = Cell::new(None);
        finish_and_submit(
            &mut preview,
            || {
                clock.set(clock.get() + Duration::from_millis(20));
                finalized.set(true);
            },
            || {
                assert!(finalized.get(), "sampled time before finalization");
                clock.get()
            },
            |delay| clock.set(clock.get() + delay),
            |()| first.set(Some(clock.get())),
        );
        let first = first.get().unwrap();
        preview.presented(first);

        clock.set(first + Duration::from_nanos(1));
        preview.acquired(clock.get());
        finalized.set(false);
        let second = Cell::new(None);
        finish_and_submit(
            &mut preview,
            || finalized.set(true),
            || {
                assert!(finalized.get(), "sampled time before finalization");
                clock.get()
            },
            |delay| clock.set(clock.get() + delay),
            |()| second.set(Some(clock.get())),
        );
        let second = second.get().unwrap();
        assert_eq!(second.duration_since(first), MIN_TRANSITION_SPACING);
        assert_eq!(preview.last_submission, Some(second));
    }

    #[test]
    fn simulated_rates_with_realistic_latency_keep_cadence_and_submission_spacing() {
        for scenario in ["short", "eight", "variable", "slow"] {
            let mut counts = Vec::new();
            for hz in [60_u64, 120, 240] {
                let start = Instant::now();
                let mut preview = DmgPreview::default();
                let mut available = start;
                let mut last_changed_submission = None;
                let mut ticks = 0_u32;
                let mut skipped = 0;
                for redraw in 0..=60 * hz {
                    let jitter = if redraw % 2 == 0 { 0 } else { 100_000 };
                    let requested =
                        start + Duration::from_nanos(redraw * 1_000_000_000 / hz + jitter);
                    // Coalesce redraw requests that arrive while the render
                    // thread is still encoding, sleeping or presenting.
                    if requested < available {
                        skipped += 1;
                        continue;
                    }
                    let acquired = requested;
                    let before = preview.model.counter();
                    preview.acquired(acquired);
                    let changed = preview.model.counter() != before;
                    let encode_us = [1_000, 2_000, 200][redraw as usize % 3];
                    let encoded = acquired + Duration::from_micros(encode_us);
                    let mut wait = Duration::ZERO;
                    preview.pace_submission(encoded, |duration| wait = duration);
                    assert!(wait <= MIN_TRANSITION_SPACING);
                    let submitted = encoded + wait;
                    if changed {
                        if let Some(last) = last_changed_submission {
                            assert!(submitted.duration_since(last) >= MIN_TRANSITION_SPACING);
                        }
                        ticks += 1;
                        last_changed_submission = Some(submitted);
                    }
                    preview.submitted(submitted);
                    let present_us = match scenario {
                        "short" => 100,
                        "eight" => 8_000,
                        "variable" => [200, 8_000, 2_000, 6_000][redraw as usize % 4],
                        "slow" => 30_000,
                        _ => unreachable!(),
                    };
                    available = submitted + Duration::from_micros(present_us);
                    preview.presented(available);
                }
                // 8 ms present latency plus encoding is sustainable at 60 Hz.
                // A continuous 30 ms present is genuinely too slow.
                if scenario == "slow" {
                    assert!((1790..=1810).contains(&ticks), "{hz} Hz: {ticks}");
                    assert!(skipped > 0);
                } else {
                    assert!(
                        (3578..=3585).contains(&ticks),
                        "{scenario}, {hz} Hz: {ticks}"
                    );
                }
                counts.push(ticks);
            }
            assert!(counts.iter().max().unwrap() - counts.iter().min().unwrap() <= 6);
        }
    }
}
