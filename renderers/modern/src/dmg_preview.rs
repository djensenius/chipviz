//! Isolated preview clock. One DMG frame is 70,224 cycles at 4,194,304 Hz.
use std::time::Instant;

use crate::scenes::dmg_tile_scroll::{Display, DmgTileScroll};

const CYCLES_PER_SECOND: u128 = 4_194_304;
const FRAME_CYCLES_NANOS: u128 = 70_224 * 1_000_000_000;

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
}

impl DmgPreview {
    /// Called only after a nonzero surface is acquired. The first acquisition
    /// runs the first ApplyDisplay/NativeStep. Later acquisitions run at most
    /// one tick; missed intervals are dropped, not replayed. The epoch and
    /// fractional phase survive failed acquisitions and resizes.
    pub(crate) fn acquired(&mut self, now: Instant) -> Display {
        let Some(epoch) = self.epoch else {
            self.epoch = Some(now);
            self.model.tick();
            return self.model.shown();
        };
        let interval = now.saturating_duration_since(epoch).as_nanos() * CYCLES_PER_SECOND
            / FRAME_CYCLES_NANOS;
        if interval > self.last_interval {
            self.last_interval = interval;
            self.model.tick();
        }
        self.model.shown()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::dmg_tile_scroll_render::shade_pixels;
    use std::time::Duration;

    #[test]
    fn startup_boundaries_and_pixel_pipeline() {
        let start = Instant::now();
        let mut preview = DmgPreview::default();
        let initial = preview.acquired(start);
        assert_eq!(
            &shade_pixels(initial)[8..16],
            &[0, 255, 255, 255, 255, 255, 255, 0]
        );
        assert_eq!(preview.model.counter(), 1);
        assert_eq!(
            preview.acquired(start + Duration::from_nanos(16_742_706)),
            initial
        );
        let next = preview.acquired(start + Duration::from_nanos(16_742_707));
        assert_eq!(preview.model.counter(), 2);
        assert_eq!(
            &shade_pixels(next)[0..8],
            &[255, 255, 255, 0, 0, 255, 255, 255]
        );
        assert_eq!(
            preview.acquired(start + Duration::from_nanos(16_742_707)),
            next
        );
    }

    #[test]
    fn failed_acquisition_resize_and_stall_drop_intervals_without_burst() {
        let start = Instant::now();
        let mut preview = DmgPreview::default();
        // A failed or zero-size acquisition never calls acquired, even at startup.
        assert_eq!(preview.model.counter(), 0);
        preview.acquired(start);
        // A failed acquisition or resize at 20 ms does not tick or reset.
        let resumed = start + Duration::from_secs(10);
        preview.acquired(resumed);
        assert_eq!(preview.model.counter(), 2);
        preview.acquired(resumed);
        assert_eq!(preview.model.counter(), 2);
        // A backwards timestamp cannot rewind the interval.
        preview.acquired(start);
        assert_eq!(preview.model.counter(), 2);
        preview.acquired(resumed + Duration::from_millis(20));
        assert_eq!(preview.model.counter(), 3);
    }

    #[test]
    fn fractional_period_does_not_accumulate_rounding_drift() {
        let start = Instant::now();
        let mut preview = DmgPreview::default();
        preview.acquired(start);
        for interval in 1..=120_u128 {
            let boundary = (interval * FRAME_CYCLES_NANOS).div_ceil(CYCLES_PER_SECOND) as u64;
            preview.acquired(start + Duration::from_nanos(boundary - 1));
            assert_eq!(preview.model.counter(), interval as u8);
            preview.acquired(start + Duration::from_nanos(boundary));
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
            for redraw in 0..=10 * hz {
                if stall && redraw > 2 * hz && redraw < 3 * hz {
                    continue;
                }
                let now = start + Duration::from_nanos(redraw * 1_000_000_000 / hz);
                let before = preview.model.counter();
                let display = preview.acquired(now);
                let advanced = preview.model.counter().wrapping_sub(before);
                assert!(advanced <= 1, "{hz} Hz, redraw {redraw}");
                ticks += u32::from(advanced);
                if stall && redraw == 3 * hz {
                    assert_eq!(advanced, 1, "resume advances exactly once");
                    assert_eq!(preview.acquired(now), display);
                    assert_eq!(preview.model.counter(), before.wrapping_add(1));
                }
                if redraw % hz == 0 && [0, 1, 2, 3, 10].contains(&(redraw / hz)) {
                    checkpoints.push((ticks, display));
                }
            }
            checkpoints
        }

        for stall in [false, true] {
            let reference = simulate(60, stall);
            let expected = if stall {
                [1, 60, 120, 121, 539]
            } else {
                [1, 60, 120, 180, 598]
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
}
