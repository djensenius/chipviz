//! The colour-field state machine from `shared/specs/colour-field-v0.md`.
//!
//! [`ColourField`] is deterministic and independent of rendering: feed every
//! received control frame to [`ColourField::observe`] (edge and onset
//! detection), then call [`ColourField::step`] once per step. The renderer
//! steps once per new `frame` counter value, at most 60 times per second.

use crate::control_frame::{ControlFrame, MAX_NOTES, Note, SPECTRUM_BANDS};

/// The scene byte that selects the colour field on every target.
pub const SCENE: u8 = 7;

const SIL_AVG: u16 = 12;
const SIL_EXIT: u8 = 24;
const SIL_HOLD: u16 = 30;
const T_FLOOR: u8 = 24;
const T_E_ON: i16 = 40;
const T_E_OFF: i16 = 16;
const T_B_ON: i16 = 56;
const T_B_OFF: i16 = 24;
const FLUX_ON: u16 = 200;
const FLUX_OFF: u16 = 64;
const NOTE_VEL: u8 = 64;
const DOM_MARGIN: u16 = 32;
const DOM_HOLD: u16 = 30;
const SECTION_MIN: u16 = 600;
const CUT_MIN: u16 = 20;
/// Indexed 1..3; index 0 is unused.
const UP: [u8; 4] = [0, 48, 104, 168];
const DOWN: [u8; 4] = [0, 32, 88, 152];
const LEVEL_OPP: u16 = 10;
const GATE_ON: u8 = 4;
const GATE_OFF_MIN: u16 = 4;
const REST_DIM_ON: u16 = 64;
const REST_DIM_OFF: u16 = 48;
const P_STABLE: u16 = 30;

const HELD: u8 = 0;
const SPECTRAL: u8 = 1;
const CUTS: u8 = 2;
const FLASH: u8 = 3;

/// Region count for a layout id (0 V2, 1 H2, 2 V3, 3 H3).
pub fn regions(layout: u8) -> u8 {
    if layout >= 2 { 3 } else { 2 }
}

/// One step's displayed state: `(mode, family, layout, combo, rot, lvl0..2)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trace {
    pub mode: u8,
    pub family: u8,
    pub layout: u8,
    pub combo: u8,
    pub rot: u8,
    pub lvl: [u8; 3],
}

/// Where `p_eff` comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteSource {
    /// Control frames: the `palette` byte goes through the `P_STABLE` gate.
    Wire,
    /// Procedural or demo frames: the `palette` byte is ignored. `armed` is
    /// the fixtures' operator pre-arming of a local 3 or 7.
    Local { p: u8, armed: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// `--allow-flash`: operator requests for 3 or 7 are armed, and a stable
    /// absolute wire 3 or 7 may arm (route b). Without it nothing arms.
    pub allow_flash: bool,
    pub palette: PaletteSource,
    /// Initial layout pin (`None` is `auto`).
    pub pin: Option<u8>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            allow_flash: false,
            palette: PaletteSource::Wire,
            pin: None,
        }
    }
}

/// Local inputs for one step. Modern only uses `pin_cycle`; the rest let the
/// fixtures model operator pulses and reduced transports.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// Operator palette pulse (operator role).
    pub operator_palette: Option<u8>,
    /// Pin control: cycles `pin_req` through auto, V2, H2, V3, H3.
    pub pin_cycle: bool,
    /// The frame's palette was reached by a relative pulse (D-pad, ring).
    pub relative_palette: bool,
    /// The frame's scene is a ring value, not carried absolutely.
    pub relative_scene: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Request {
    p: u8,
    armed: bool,
    wire: bool,
}

/// `(effective mode, family)`: an unarmed 3 runs as 2.
fn effective(p: u8, armed: bool) -> (u8, u8) {
    let mode = p & 3;
    let mode = if mode == FLASH && !armed { CUTS } else { mode };
    (mode, (p >> 2) & 1)
}

#[derive(Clone, Copy, Debug, Default)]
struct Events {
    beat_bar: bool,
    drop: bool,
    note: bool,
}

/// Per-frame edge and onset detection against the previously received frame.
#[derive(Clone, Copy, Debug, Default)]
struct Onsets {
    previous: Option<(u8, [Note; MAX_NOTES], usize)>,
    pending: Events,
}

impl Onsets {
    fn observe(&mut self, frame: &ControlFrame) {
        let count = usize::from(frame.note_count).min(MAX_NOTES);
        if let Some((flags, notes, previous_count)) = self.previous {
            let rising = frame.flags & !flags;
            self.pending.beat_bar |= rising & 0x03 != 0;
            self.pending.drop |= rising & 0x04 != 0;
            self.pending.note |= frame.notes[..count].iter().any(|note| {
                note.velocity >= NOTE_VEL
                    && !notes[..previous_count]
                        .iter()
                        .any(|old| old.note == note.note && old.velocity == note.velocity)
            });
        }
        self.previous = Some((frame.flags, frame.notes, count));
    }

    fn take(&mut self) -> Events {
        std::mem::take(&mut self.pending)
    }
}

/// Wire palette stability gate (`P_STABLE`) with the scene-7 window.
#[derive(Clone, Copy, Debug)]
struct WireGate {
    value: u8,
    count: u16,
    consumed: bool,
    /// The current run began with a relative pulse.
    relative: bool,
    /// Every counted step carried an absolute scene 7.
    clean: bool,
}

/// State for one visit to scene 7.
#[derive(Clone, Debug)]
struct Field {
    acc: [u16; 4],
    prev_spec: [u8; SPECTRUM_BANDS],
    t_armed: bool,
    silent: bool,
    quiet: u16,
    dom: u8,
    cand: Option<u8>,
    cand_n: u16,
    layout: u8,
    combo: u8,
    rot: u8,
    p_eff: u8,
    flash: bool,
    act: u8,
    since_cut: u16,
    since_layout: u16,
    gate_off: u16,
    since_pulse: u16,
    lg: u16,
    lvl: [u8; 3],
    pulse: u8,
    hyst: bool,
    dim: bool,
    p_req: Option<Request>,
    wire: WireGate,
}

fn inputs(frame: &ControlFrame) -> [u8; 4] {
    [frame.energy, frame.bass, frame.mid, frame.treble]
}

/// Ties go to the lowest index.
fn argmax(values: [u16; 3]) -> u8 {
    let mut best = 0;
    for index in 1..3 {
        if values[index] > values[best] {
            best = index;
        }
    }
    best as u8
}

fn level_for(value: u8) -> u8 {
    UP[1..].iter().filter(|&&up| up <= value).count() as u8
}

impl Field {
    fn enter(
        frame: &ControlFrame,
        config: &Config,
        pin: Option<u8>,
        power_on: bool,
        controls: Controls,
    ) -> Self {
        let (p_eff, flash) = match config.palette {
            PaletteSource::Wire => (frame.palette & 7, false),
            PaletteSource::Local { p, armed } => (p & 7, armed && p & 3 == FLASH),
        };
        let bands = [frame.bass, frame.mid, frame.treble].map(u16::from);
        let mut field = Self {
            acc: inputs(frame).map(|value| u16::from(value) << 3),
            prev_spec: frame.spectrum,
            t_armed: true,
            silent: false,
            quiet: 0,
            dom: argmax(bands),
            cand: None,
            cand_n: 0,
            layout: pin.unwrap_or(0),
            combo: 0,
            rot: 0,
            p_eff,
            flash,
            act: 0,
            since_cut: 0,
            since_layout: 0,
            gate_off: GATE_OFF_MIN,
            since_pulse: u16::from(GATE_ON) + GATE_OFF_MIN,
            lg: 0,
            lvl: [0; 3],
            pulse: 0,
            hyst: false,
            dim: false,
            p_req: None,
            wire: WireGate {
                value: frame.palette & 7,
                count: u16::from(frame.scene == SCENE),
                consumed: false,
                relative: controls.relative_palette,
                clean: !controls.relative_scene,
            },
        };
        field.mode_entry(frame, power_on);
        field
    }

    fn avg(&self, index: usize) -> u16 {
        self.acc[index] >> 3
    }

    fn mode(&self) -> (u8, u8) {
        effective(self.p_eff, self.flash)
    }

    fn band_values(&self, frame: &ControlFrame) -> [u8; 3] {
        if regions(self.layout) == 3 {
            [frame.bass, frame.mid, frame.treble]
        } else {
            [frame.bass, frame.mid.max(frame.treble), 0]
        }
    }

    fn init_levels(&mut self, frame: &ControlFrame) {
        self.lvl = self.band_values(frame).map(level_for);
        if regions(self.layout) == 2 {
            self.lvl[2] = 0;
        }
    }

    fn mode_entry(&mut self, frame: &ControlFrame, power_on: bool) {
        self.act = 0;
        (self.since_cut, self.lg) = if power_on {
            (CUT_MIN, LEVEL_OPP)
        } else {
            (0, 0)
        };
        let (mode, _) = self.mode();
        if mode == HELD {
            self.combo = self.dom;
        }
        self.init_levels(frame);
        self.pulse = 0;
        self.gate_off = GATE_OFF_MIN;
        self.hyst = self.avg(0) >= REST_DIM_ON;
        self.dim = self.hyst;
        if self.silent && (mode == SPECTRAL || mode == FLASH) {
            self.lvl = [0; 3];
            self.hyst = false;
            self.dim = false;
        }
    }

    /// Every visible change outside armed flash/gate resets `lg`.
    fn visible_change(&mut self) {
        if self.mode().0 != FLASH {
            self.lg = 0;
        }
    }

    fn next_layout(&mut self, pin: Option<u8>, frame: &ControlFrame) {
        if pin.is_none() {
            self.set_layout((self.layout + 1) & 3, frame);
        }
    }

    fn set_layout(&mut self, layout: u8, frame: &ControlFrame) {
        self.layout = layout;
        self.rot %= regions(layout);
        self.since_layout = 0;
        if self.mode().0 == SPECTRAL {
            self.init_levels(frame);
        }
        self.visible_change();
    }

    fn next_combo(&mut self) {
        self.combo = (self.combo + 1) & 3;
        self.visible_change();
    }

    fn rotate(&mut self) {
        self.rot = (self.rot + 1) % regions(self.layout);
        self.visible_change();
    }

    /// Steps 1 to 6 of every step after the entry step.
    fn rules(&mut self, frame: &ControlFrame, events: Events, pin: Option<u8>) {
        // 1. Deltas against the previous averages, then update them.
        let values = inputs(frame);
        let delta = |index: usize| i16::from(values[index]) - self.avg(index) as i16;
        let de = delta(0);
        let db = [delta(1), delta(2), delta(3)];
        let flux = frame
            .spectrum
            .iter()
            .zip(self.prev_spec)
            .map(|(&now, before)| u16::from(now.saturating_sub(before)))
            .sum::<u16>();
        for (acc, value) in self.acc.iter_mut().zip(values) {
            *acc = (i32::from(*acc) + i32::from(value) - i32::from(*acc >> 3)) as u16;
        }
        self.prev_spec = frame.spectrum;

        // 2. Silence, and the budgeted spectral blackout.
        let (mode, _) = self.mode();
        if self.silent {
            if frame.energy >= SIL_EXIT {
                self.silent = false;
                self.quiet = 0;
            }
        } else {
            self.quiet = if self.avg(0) <= SIL_AVG {
                self.quiet.saturating_add(1)
            } else {
                0
            };
            if self.quiet >= SIL_HOLD {
                self.silent = true;
                self.dim = false;
                if self.pulse > 0 {
                    self.pulse = 0;
                    self.gate_off = 0;
                }
            }
        }
        if mode == SPECTRAL && self.silent && self.lvl != [0; 3] && self.lg >= LEVEL_OPP {
            self.lvl = [0; 3];
            self.lg = 0;
        }

        // 3. Transients.
        let hit = frame.energy >= T_FLOOR
            && (de >= T_E_ON || db.iter().any(|&delta| delta >= T_B_ON) || flux >= FLUX_ON);
        let calm = de < T_E_OFF && db.iter().all(|&delta| delta < T_B_OFF) && flux < FLUX_OFF;
        let mut transient = false;
        if self.t_armed && hit {
            transient = true;
            self.t_armed = false;
        } else if calm {
            self.t_armed = true;
        }

        // 4. Events.
        let x = events.drop;
        let e = !self.silent && (transient || events.note || events.beat_bar || x);

        // 5. Dominance.
        let averages = [self.avg(1), self.avg(2), self.avg(3)];
        let c = argmax(averages);
        let mut d = false;
        if !self.silent
            && c != self.dom
            && averages[usize::from(c)] >= averages[usize::from(self.dom)] + DOM_MARGIN
        {
            self.cand_n = if self.cand == Some(c) {
                self.cand_n.saturating_add(1)
            } else {
                1
            };
            self.cand = Some(c);
        } else {
            self.cand = None;
            self.cand_n = 0;
        }
        if self.cand_n >= DOM_HOLD {
            self.dom = c;
            self.cand_n = 0;
            d = true;
        }

        // 6. Mode rule.
        if self.silent {
            return;
        }
        let section = x || (d && self.since_layout >= SECTION_MIN);
        match mode {
            HELD => {
                if d && self.combo != self.dom {
                    self.combo = self.dom;
                    self.lg = 0;
                }
                if section && self.since_cut >= CUT_MIN {
                    self.next_layout(pin, frame);
                    self.since_cut = 0;
                }
            }
            SPECTRAL => {
                if x && self.since_cut >= CUT_MIN && self.lg >= LEVEL_OPP {
                    self.next_layout(pin, frame);
                    self.next_combo();
                    self.since_cut = 0;
                } else if d && self.since_layout >= SECTION_MIN && self.lg >= LEVEL_OPP {
                    self.next_layout(pin, frame);
                }
                if self.lg >= LEVEL_OPP {
                    self.update_levels(frame);
                }
            }
            CUTS => {
                if e && self.since_cut >= CUT_MIN {
                    self.since_cut = 0;
                    self.lg = 0;
                    if x {
                        self.next_layout(pin, frame);
                        self.next_combo();
                    } else {
                        match self.act {
                            1 => self.next_combo(),
                            3 if pin.is_none() => self.next_layout(pin, frame),
                            _ => self.rotate(),
                        }
                        self.act = (self.act + 1) & 3;
                    }
                }
            }
            _ => self.gate(e, x),
        }
    }

    /// Spectral level update: at most one step per region, field-wide `lg`.
    fn update_levels(&mut self, frame: &ControlFrame) {
        let values = self.band_values(frame);
        let count = usize::from(regions(self.layout));
        let mut changed = false;
        for (level, value) in self.lvl.iter_mut().zip(values).take(count) {
            if *level < 3 && value >= UP[usize::from(*level + 1)] {
                *level += 1;
                changed = true;
            } else if *level > 0 && value < DOWN[usize::from(*level)] {
                *level -= 1;
                changed = true;
            }
        }
        if changed {
            self.lg = 0;
        }
    }

    /// Flash/gate (mode 3, armed only).
    fn gate(&mut self, e: bool, x: bool) {
        let average = self.avg(0);
        if average >= REST_DIM_ON {
            self.hyst = true;
        } else if average < REST_DIM_OFF {
            self.hyst = false;
        }
        if self.pulse > 0 {
            self.pulse -= 1;
            if self.pulse == 0 {
                self.gate_off = 0;
                self.dim = self.hyst;
            }
        } else if e
            && self.gate_off >= GATE_OFF_MIN
            && self.since_pulse >= u16::from(GATE_ON) + GATE_OFF_MIN
        {
            self.pulse = GATE_ON;
            self.since_pulse = 0;
            self.rotate();
            if x {
                self.next_combo();
            }
        }
    }

    /// Keeps only the latest request; one that matches the current effective
    /// mode and family is cleared, discarding its arming.
    fn request(&mut self, request: Request) {
        let current = self.mode();
        self.p_req = (effective(request.p, request.armed) != current).then_some(request);
    }

    /// The wire stability gate (step 7 only) and operator pulses.
    fn update_requests(
        &mut self,
        frame: &ControlFrame,
        config: &Config,
        controls: Controls,
        gate: bool,
    ) {
        if gate && config.palette == PaletteSource::Wire {
            let value = frame.palette & 7;
            if frame.scene != SCENE {
                // Outside the scene-7 window: restart the count and drop any
                // pending wire request with its arming.
                self.wire.count = 0;
                if self.p_req.is_some_and(|request| request.wire) {
                    self.p_req = None;
                }
            } else {
                let wire = &mut self.wire;
                if value != wire.value {
                    *wire = WireGate {
                        value,
                        count: 0,
                        consumed: false,
                        relative: controls.relative_palette,
                        clean: true,
                    };
                }
                if wire.count == 0 {
                    wire.clean = true;
                }
                wire.count = wire.count.saturating_add(1);
                wire.clean &= !controls.relative_scene;
                if wire.count >= P_STABLE && !wire.consumed {
                    wire.consumed = true;
                    let armed =
                        config.allow_flash && value & 3 == FLASH && !wire.relative && wire.clean;
                    self.request(Request {
                        p: value,
                        armed,
                        wire: true,
                    });
                }
            }
        }
        if let Some(p) = controls.operator_palette {
            let p = p & 7;
            self.wire.consumed = true;
            self.request(Request {
                p,
                armed: config.allow_flash && p & 3 == FLASH,
                wire: false,
            });
        }
    }

    fn trace(&self) -> Trace {
        let (mode, family) = self.mode();
        let mut lvl = match mode {
            SPECTRAL => self.lvl,
            FLASH if self.pulse > 0 => [3; 3],
            FLASH => [u8::from(self.dim); 3],
            _ => [3; 3],
        };
        lvl[usize::from(regions(self.layout))..].fill(0);
        Trace {
            mode,
            family,
            layout: self.layout,
            combo: self.combo,
            rot: self.rot,
            lvl,
        }
    }
}

/// What the latest step shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    /// A native registry scene: the debounced scene byte, never 7.
    Native(u8),
    ColourField(Trace),
}

/// The colour field plus the scene-7 debounce, onset accumulation and pin.
#[derive(Clone, Debug)]
pub struct ColourField {
    config: Config,
    onsets: Onsets,
    newest: Option<ControlFrame>,
    last_stepped: Option<u16>,
    steps: u64,
    shown: bool,
    entry_pending: bool,
    native: u8,
    scene_value: u8,
    scene_run: u16,
    since_scene_change: u16,
    pin: Option<u8>,
    pin_req: Option<Option<u8>>,
    field: Option<Field>,
    display: Display,
}

impl ColourField {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            onsets: Onsets::default(),
            newest: None,
            last_stepped: None,
            steps: 0,
            shown: false,
            entry_pending: false,
            native: 0,
            scene_value: 0,
            scene_run: 0,
            since_scene_change: u16::MAX,
            pin: config.pin.map(|layout| layout & 3),
            pin_req: None,
            field: None,
            display: Display::Native(0),
        }
    }

    pub fn config(&self) -> Config {
        self.config
    }

    /// Edge and onset detection for one received frame, against the frame
    /// received before it. Re-observing an identical frame raises nothing.
    pub fn observe(&mut self, frame: &ControlFrame) {
        self.onsets.observe(frame);
        self.newest = Some(*frame);
    }

    /// True when the newest observed frame's `frame` counter has not been
    /// stepped yet.
    pub fn has_new_frame(&self) -> bool {
        self.newest
            .is_some_and(|frame| self.last_stepped != Some(frame.frame))
    }

    /// What the latest step shows.
    pub fn display(&self) -> Display {
        self.display
    }

    #[cfg(test)]
    pub(super) fn is_silent(&self) -> bool {
        self.field.as_ref().is_some_and(|field| field.silent)
    }

    /// Runs one step on the newest observed frame and returns its trace
    /// (`None` while a native scene is shown).
    pub fn step(&mut self, controls: Controls) -> Option<Trace> {
        let frame = self.newest.unwrap_or(ControlFrame {
            energy: 0,
            bass: 0,
            mid: 0,
            treble: 0,
            spectrum: [0; SPECTRUM_BANDS],
            scene: 0,
            palette: 0,
            flags: 0,
            note_count: 0,
            ..ControlFrame::procedural(0)
        });
        self.last_stepped = Some(frame.frame);
        let events = self.onsets.take();
        let power_on = self.steps == 0;
        self.steps = self.steps.saturating_add(1);
        if controls.pin_cycle {
            self.pin_req = Some(match self.pin_req.unwrap_or(self.pin) {
                None => Some(0),
                Some(3) => None,
                Some(layout) => Some(layout + 1),
            });
        }
        if power_on && frame.scene == SCENE {
            // Power-on entry is exempt from the debounce.
            self.shown = true;
            self.entry_pending = true;
        }

        let entering = std::mem::take(&mut self.entry_pending);
        let trace = if !self.shown {
            if frame.scene != SCENE {
                self.native = frame.scene;
            }
            None
        } else if entering {
            let field = self.field.insert(Field::enter(
                &frame,
                &self.config,
                self.pin,
                power_on,
                controls,
            ));
            field.update_requests(&frame, &self.config, controls, false);
            Some(field.trace())
        } else {
            let field = self.field.as_mut().expect("scene 7 is shown");
            field.rules(&frame, events, self.pin);
            let trace = field.trace();
            // 7. Counters, then the wire gate and operator pulses.
            for counter in [
                &mut field.since_cut,
                &mut field.since_layout,
                &mut field.gate_off,
                &mut field.since_pulse,
                &mut field.lg,
            ] {
                *counter = counter.saturating_add(1);
            }
            field.update_requests(&frame, &self.config, controls, true);
            Some(trace)
        };
        self.display = match trace {
            Some(trace) => Display::ColourField(trace),
            None => Display::Native(self.native),
        };
        self.apply_requests(&frame, trace.is_some() && !entering);
        self.debounce(frame.scene, power_on);
        trace
    }

    /// Applies at most one pending request under the change budget.
    fn apply_requests(&mut self, frame: &ControlFrame, budget_step: bool) {
        let Some(field) = self.field.as_mut() else {
            if let Some(pin) = self.pin_req.take() {
                self.pin = pin;
            }
            return;
        };
        // Auto, or pinning the layout already shown, needs no budget.
        if let Some(pin) = self.pin_req
            && pin.is_none_or(|layout| layout == field.layout)
        {
            self.pin = pin;
            self.pin_req = None;
        }
        if !budget_step || field.lg < LEVEL_OPP || field.since_cut < CUT_MIN {
            return;
        }
        if let Some(request) = field.p_req.take() {
            field.p_eff = request.p;
            field.flash = request.armed && request.p & 3 == FLASH;
            field.mode_entry(frame, false);
        } else if let Some(Some(layout)) = self.pin_req.take() {
            self.pin = Some(layout);
            field.set_layout(layout, frame);
            field.lg = 0;
            field.since_cut = 0;
        }
    }

    /// The scene-7 debounce on the wire scene value.
    fn debounce(&mut self, scene: u8, power_on: bool) {
        if !power_on && scene == self.scene_value {
            self.scene_run = self.scene_run.saturating_add(1);
        } else {
            self.scene_value = scene;
            self.scene_run = 1;
        }
        self.since_scene_change = self.since_scene_change.saturating_add(1);
        let wants = scene == SCENE;
        if wants != self.shown && self.scene_run >= P_STABLE && self.since_scene_change >= CUT_MIN {
            self.shown = wants;
            self.since_scene_change = 0;
            if wants {
                self.entry_pending = true;
            } else {
                // Leaving scene 7 clears the flash latch with the field.
                self.native = scene;
                self.field = None;
            }
        }
    }
}
