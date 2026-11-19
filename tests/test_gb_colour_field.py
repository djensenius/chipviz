"""Deterministic SameBoy (libretro) checks for the Game Boy colour field.

Fixture tests run the CHIPVIZ_FIXTURE build, which takes one full-path
control frame per step from a WRAM mailbox, and compare the trace with the
Expected column of shared/specs/colour-field-v0.md. Pad tests run the default
data and operator ROMs and check every pixel of captures against the spec's
DMG geometry and shades. The tests skip when RGBDS or the SameBoy libretro
core is not installed.
"""

import pathlib
import shutil
import subprocess
import sys
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import gb_libretro_capture as glc


HOMEBREW = ROOT / "cores" / "gb" / "homebrew"
CAPTURES = ROOT / "build" / "gb-colour-field"
CORE = glc.default_core()
TOOLS = shutil.which("rgbasm") and shutil.which("rgblink") and shutil.which("rgbfix") and shutil.which("make")

# WRAM addresses (cores/gb/homebrew/colour_field.asm).
TRACE, SHOWN, STEP, ACK, READY = 0xC000, 0xC009, 0xC012, 0xC014, 0xC015
SEQ, CTL, LOCAL_P, ARMED, PIN, SCENE, PAL, OP_REQ, OP_P, OP_ARMED, FX_IN = range(0xC020, 0xC02B)
CTL_WIRE, CTL_PAD = 1, 2
BEAT, BAR, DROP = 1, 2, 4
PIN_AUTO = 0xFF

# Spec DMG table: shade per region entry for each combo, and the grey levels.
COMBOS = {2: [(0, 3), (1, 3), (0, 2), (3, 1)], 3: [(0, 3, 1), (3, 0, 2), (1, 3, 0), (0, 2, 3)]}
GREY = {0: (255, 255, 255), 1: (170, 170, 170), 2: (85, 85, 85), 3: (0, 0, 0)}
V2, H2, V3, H3 = range(4)


def regions(layout):
  return 3 if layout >= 2 else 2


def region_of(layout, x, y):
  """Spec geometry, g = 8: columns from the left, rows from the bottom."""
  n = regions(layout)
  size, pos = (160, x) if layout in (V2, V3) else (144, 143 - y)
  bounds = [8 * ((i * (size // 8)) // n) for i in range(n)] + [size]
  return next(i for i in range(n) if bounds[i] <= pos < bounds[i + 1])


def expected_shades(trace):
  """Spec shades: combo at level 3, 3 - L for spectral, gate rests 2 and 3."""
  mode, _, layout, combo, rot, *levels = trace
  n = regions(layout)
  out = []
  for i in range(n):
    if mode == 1:
      out.append(3 - levels[i])
    elif levels[i] == 3:
      out.append(COMBOS[n][combo][(i + rot) % n])
    else:
      out.append({1: 2, 0: 3}[levels[i]])
  return out


def frame_in(e=0, bands=(0, 0, 0), spectrum=(0,) * 8, flags=0, notes=()):
  data = bytes([e, *bands, *spectrum, flags, len(notes)])
  data += b"".join(bytes(n) for n in notes)
  return data.ljust(22, b"\0")


def flat(value, **kw):
  return frame_in(value, (value,) * 3, (value,) * 8, **kw)


class Lcg:
  def __init__(self):
    self.x = 1

  def next(self):
    self.x = (self.x * 25173 + 13849) & 0xFFFF
    return self.x


class Rom:
  """One ROM in the SameBoy core; at most one is loaded at a time."""

  def __init__(self, path, video=False, symbols=None):
    self.gb = glc.GbCore(path, CORE, video=video)
    self.symbols = {}
    if symbols:
      for line in symbols.read_text().splitlines():
        bank, _, rest = line.partition(":")
        if bank == "00" and " " in rest:
          address, name = rest.split()
          self.symbols[name] = int(address, 16)

  def boot(self):
    self.gb.write(READY, b"\0")
    self.gb.reset()
    for _ in range(600):
      self.gb.run_frame()
      if self.gb.read(READY)[0] == 0xA5:
        return
    raise AssertionError("ROM never became ready")

  def close(self):
    self.gb.close()


class Fixture(Rom):
  """Full-path control frames from the mailbox, one step per frame counter."""

  def run(self, inputs, steps=600, p=0, armed=None, wire=False, pad=False, pin=PIN_AUTO, scene=7, pal=0, probe=()):
    """inputs(t) gives the frame bytes, or a dict of mailbox overrides.

    probe names WRAM variables recorded after each step in self.probes.
    """
    self.boot()
    self.probes = {name: [] for name in probe}
    armed = p in (3, 7) if armed is None else armed
    self.gb.write(CTL, bytes([(CTL_WIRE if wire else 0) | (CTL_PAD if pad else 0), p, armed, pin, scene, pal]))
    seq = self.gb.read(SEQ)[0]
    trace = []
    for t in range(steps):
      step = inputs(t)
      if isinstance(step, bytes):
        step = {"in": step}
      self.gb.write(FX_IN, step.get("in", bytes(22)))
      self.gb.write(SCENE, bytes([step.get("scene", scene)]))
      self.gb.write(PAL, bytes([step.get("pal", pal)]))
      op = step.get("op")
      self.gb.write(OP_REQ, bytes([1, op[0], op[1]]) if op else b"\0\0\0")
      if pad:
        # The core latches the pad a frame before the step reads it; this
        # frame runs no step because the counter is unchanged.
        self.gb.run_frame(step.get("buttons", ()))
      seq = (seq + 1) & 0xFF
      self.gb.write(SEQ, bytes([seq]))
      for _ in range(4):
        self.gb.run_frame(step.get("buttons", ()))
        if self.gb.read(ACK)[0] == seq:
          break
      else:
        raise AssertionError(f"step {t} not acknowledged")
      raw = self.gb.read(TRACE, 9)
      trace.append(tuple(raw[1:]) if raw[0] == 1 else None)
      for name, values in self.probes.items():
        values.append(self.gb.read(self.symbols[name])[0])
    return trace


def changes(trace):
  return [t for t in range(1, len(trace)) if trace[t] != trace[t - 1]]


def pulses(trace):
  """Pulse starts: steps whose displayed level becomes 3 (flash/gate)."""
  return [t for t in range(1, len(trace)) if trace[t][5] == 3 and trace[t - 1][5] != 3]


def max_in_window(steps, width=60):
  return max((sum(1 for s in steps if t <= s < t + width) for t in range(600)), default=0)


@unittest.skipUnless(TOOLS and CORE.exists(), "needs RGBDS and the SameBoy libretro core")
class GbColourFieldFixtures(unittest.TestCase):
  """F0..F24 (F23 is host-only); F15 and the resampled F18 path need a 120 Hz
  source, which the DMG target does not have."""

  @classmethod
  def setUpClass(cls):
    subprocess.run(["make", "-s", "-C", str(HOMEBREW), "FIXTURE=1"], check=True, capture_output=True)
    cls.rom = Fixture(HOMEBREW / "chipviz-gb-fixture.gb", symbols=HOMEBREW / "build" / "fixture" / "chipviz-gb.sym")

  @classmethod
  def tearDownClass(cls):
    cls.rom.close()

  def modes(self, inputs, ps=range(8), **kw):
    return {p: self.rom.run(inputs, p=p, **kw) for p in ps}

  def assert_family_pair(self, runs):
    """Family B (p 4..7) has the same trace as family A apart from the family."""
    for p in range(4):
      if p in runs and p + 4 in runs:
        self.assertEqual([(m, l, c, r, *v) for m, _, l, c, r, *v in runs[p]],
                         [(m, l, c, r, *v) for m, _, l, c, r, *v in runs[p + 4]], f"p {p} vs {p + 4}")

  def test_f0_silence(self):
    runs = self.modes(lambda t: bytes(22))
    for p, trace in runs.items():
      self.assertEqual(changes(trace), [], p)
      levels = (3, 3, 0) if p % 4 in (0, 2) else (0, 0, 0)
      self.assertEqual(trace[0], (p % 4, p // 4, V2, 0, 0, *levels), p)

  def test_f1_tone_drop(self):
    def inputs(t):
      return frame_in(160, (40, 200, 40), (40, 40, 120, 200, 200, 120, 40, 40), DROP if 300 <= t <= 305 else 0)
    runs = self.modes(inputs)
    self.assert_family_pair(runs)
    held, spectral, cuts, gate = (runs[p] for p in range(4))
    self.assertEqual(held[0][3], 1)
    self.assertEqual(changes(held), [300])
    self.assertEqual(held[300][2:4], (H2, 1))
    self.assertEqual(spectral[0][5:7], (0, 3))
    self.assertEqual(spectral[300][2:4], (H2, 1))
    self.assertEqual(cuts[300][2:4], (H2, 1))
    self.assertEqual(changes(cuts), [300])
    self.assertEqual(pulses(gate), [300])
    self.assertEqual(gate[300][3:6], (1, 1, 3))

  def test_f2_noise(self):
    lcg = Lcg()
    table = [bytes(150 + ((lcg.next() >> 8) % 25) - 12 for _ in range(12)) for _ in range(600)]
    runs = self.modes(lambda t: frame_in(table[t][0], table[t][1:4], table[t][4:12]))
    for p, trace in runs.items():
      self.assertEqual(changes(trace), [], p)
    self.assertEqual(runs[1][0][5:7], (2, 2))
    self.assertEqual(runs[3][0][5:7], (1, 1))

  def test_f3_aperiodic(self):
    spikes = {37, 101, 113, 250, 400, 523}
    runs = self.modes(lambda t: flat(220 if t in spikes or t - 1 in spikes else 40), ps=(0, 2, 3))
    self.assertEqual(changes(runs[0]), [])
    self.assertEqual(changes(runs[2]), [37, 101, 250, 400, 523])
    self.assertEqual(pulses(runs[3]), sorted(spikes))

  def test_f4_notes(self):
    def inputs(t):
      notes = [(60, 100)] if t in (45, 46, 130, 352) else [(60, 40)] if t == 200 else []
      return flat(80, notes=notes)
    runs = self.modes(inputs, ps=(0, 2, 3))
    self.assertEqual(changes(runs[0]), [])
    self.assertEqual(changes(runs[2]), [45, 130, 352])
    self.assertEqual(pulses(runs[3]), [45, 130, 352])

  def f5_inputs(self, t):
    beat = next((b for b in range(30, 600, 30) if b <= t <= b + 3), None)
    flags = (BEAT if t % 30 == 0 and t else 0) | (BAR if t % 120 == 0 and t else 0)
    if beat is not None:
      return frame_in(180, (200, 60, 60), (200, 200, 60, 60, 60, 60, 60, 60), flags)
    return flat(60, flags=flags)

  def test_f5_rhythmic(self):
    runs = self.modes(self.f5_inputs, ps=(0, 2, 3))
    self.assertEqual(changes(runs[0]), [])
    cuts = runs[2]
    self.assertEqual(changes(cuts), list(range(30, 600, 30)))
    layouts = [t for t in changes(cuts) if cuts[t][2] != cuts[t - 1][2]]
    self.assertEqual(layouts, [120, 240, 360, 480])
    self.assertEqual([cuts[t][2] for t in layouts], [H2, V3, H3, V2])
    self.assertEqual(len(pulses(runs[3])), 19)

  def test_f6_floor(self):
    lcg = Lcg()
    table = []
    for t in range(600):
      floor = [(lcg.next() >> 8) % 13 for _ in range(12)] if 60 <= t <= 419 else None
      if floor:
        table.append(frame_in(floor[0], floor[1:4], floor[4:12]))
      else:
        table.append(flat(220 if t in (480, 481) else 120))
    runs = self.modes(lambda t: table[t], ps=(1, 2, 3))
    spectral = runs[1]
    self.assertEqual([(t, spectral[t][5]) for t in changes(spectral)],
                     [(60, 1), (70, 0), (420, 1), (430, 2), (480, 3), (490, 2)])
    self.assertEqual(changes(runs[2]), [420, 480])
    self.assertEqual(pulses(runs[3]), [420, 480])
    for p in (1, 2):
      self.assertTrue(all(runs[p][t] == runs[p][80] for t in range(80, 420)), p)
    gate = runs[3]  # resting dim until silence entry, then dark
    entry = [t for t in changes(gate) if 80 <= t < 420]
    self.assertEqual(len(entry), 1)
    self.assertLess(entry[0], 130)
    self.assertEqual((gate[entry[0] - 1][5], gate[entry[0]][5]), (1, 0))

  def test_f7_sweep(self):
    def inputs(t):
      if t < 300:
        return frame_in(150, (200, 60, 60), (200,) * 4 + (60,) * 4)
      return frame_in(150, (60, 60, 200), (60,) * 4 + (200,) * 4)
    runs = self.modes(inputs, ps=(0, 1, 2, 3))
    held = changes(runs[0])
    self.assertEqual(len(held), 1)
    self.assertTrue(330 <= held[0] <= 345)
    self.assertEqual((runs[0][0][3], runs[0][-1][3], runs[0][-1][2]), (0, 2, V2))
    spectral = runs[1]
    self.assertEqual(spectral[0][5:7], (3, 1))
    self.assertEqual([(t, spectral[t][5:7]) for t in changes(spectral)], [(300, (2, 2)), (310, (1, 3))])
    self.assertEqual(changes(runs[2]), [300])
    self.assertEqual(pulses(runs[3]), [300])

  def test_f8_held_note(self):
    def inputs(t):
      notes = [(60, 100)] if 100 <= t <= 299 else []
      if 200 <= t <= 299:
        notes.append((64, 100))
      return flat(80, notes=notes)
    runs = self.modes(inputs, ps=(2, 3))
    self.assertEqual(changes(runs[2]), [100, 200])
    self.assertEqual(pulses(runs[3]), [100, 200])

  def test_f9_beat_bar(self):
    beats = {20 + 40 * k for k in range(15)}
    bars = {40 + 120 * k for k in range(5)}
    runs = self.modes(lambda t: flat(100, flags=(BEAT if t in beats else 0) | (BAR if t in bars else 0)), ps=(0, 2, 3))
    self.assertEqual(changes(runs[0]), [])
    self.assertEqual(changes(runs[2]), sorted(beats | bars))
    self.assertEqual(pulses(runs[3]), sorted(beats | bars))

  def test_f10_wire_gate(self):
    def inputs(t):
      pal = 2 if 100 <= t <= 109 else 3 if t >= 200 else 0
      return {"in": frame_in(160, (40, 200, 40), (40, 40, 120, 200, 200, 120, 40, 40)), "pal": pal}
    trace = self.rom.run(inputs, wire=True)
    self.assertTrue(all(s[0] == 0 for s in trace[:230]))
    self.assertTrue(all(s[0] == 3 for s in trace[230:]))
    self.assertEqual(trace[230][5:7], (1, 1))

  def test_f11_pinned(self):
    trace = self.rom.run(self.f5_inputs, p=2, pin=H3)
    self.assertTrue(all(s[2] == H3 for s in trace))
    cuts = changes(trace)
    self.assertEqual(len(cuts), 19)
    rotations = [t for t in cuts if trace[t][3] == trace[t - 1][3]]
    self.assertEqual(len(rotations), 19 - 5)  # combo changes at actions 1 only

  def test_f12_mode_toggle(self):
    def inputs(t):
      if t <= 594 and t >= 60 and (t - 60) % 12 == 0:
        return {"in": bytes(22), "op": (1, 0)}
      if t <= 594 and t >= 66 and (t - 66) % 12 == 0:
        return {"in": bytes(22), "op": (0, 0)}
      return bytes(22)
    trace = self.rom.run(inputs)
    applied = [t - 1 for t in changes(trace)]
    self.assertEqual(applied[:7], [60, 80, 100, 126, 146, 166, 192])
    self.assertTrue(all(b - a in (20, 26) for a, b in zip(applied, applied[1:])))
    for t in changes(trace):
      self.assertIn(trace[t], [(0, 0, V2, 0, 0, 3, 3, 0), (1, 0, V2, 0, 0, 0, 0, 0)])
    self.assertLessEqual(max_in_window(changes(trace)), 3)

  def test_f13_silence_spikes(self):
    runs = self.modes(lambda t: frame_in(30 if t % 31 == 0 and t else 0, (250,) * 3), ps=(0, 1, 2, 3))
    for p in (0, 2, 3):
      self.assertEqual(changes(runs[p]), [], p)
    spectral = runs[1]
    levels = [(t, spectral[t][5]) for t in changes(spectral)]
    self.assertEqual(levels[:9], [(30, 0), (40, 1), (50, 2), (60, 3), (92, 0), (102, 1), (112, 2), (122, 3), (154, 0)])
    self.assertLessEqual(max_in_window(changes(spectral)), 4)

  def test_f14_dim_rest_attack(self):
    runs = self.modes(lambda t: frame_in(255 if t % 5 == 0 else 0), ps=(0, 1, 2, 3))
    self.assertEqual(changes(runs[0]), [])
    self.assertEqual(changes(runs[1]), [])
    self.assertEqual(changes(runs[2]), list(range(5, 600, 20)))
    gate = runs[3]
    self.assertEqual(pulses(gate), list(range(5, 600, 10)))
    self.assertTrue(all(gate[t][5] in (1, 3) for t in range(29)))
    rest = [gate[t][5] for t in range(30, 600)]
    self.assertEqual(rest, ([0, 0, 0, 0, 0, 3, 3, 3, 3, 0] * 57)[:570])
    self.assertLessEqual(max_in_window(changes(gate)), 12)

  def test_f16_armed_toggle(self):
    def inputs(t):
      frame = flat(100, flags=BEAT if t % 2 else 0)
      if t >= 98 and (t - 98) % 60 == 0:
        return {"in": frame, "op": (7, 1)}
      if t >= 128 and (t - 128) % 60 == 0:
        return {"in": frame, "op": (3, 1)}
      return frame
    trace = self.rom.run(inputs, p=3)
    applied = [t - 1 for t in range(1, 600) if trace[t][1] != trace[t - 1][1]]
    self.assertEqual(applied, list(range(98, 580, 30)))
    self.assertEqual(pulses(trace), [1 + 8 * k for k in range(75)])

  def test_f17_silence_cut_off(self):
    trace = self.rom.run(lambda t: frame_in(24 if t % 31 == 0 and t else 0, flags=BEAT if t % 2 else 0), p=3,
                         probe=("wSilent", "wPulse"))
    silent, pulse = self.rom.probes["wSilent"], self.rom.probes["wPulse"]
    entries = [t for t in range(1, 600) if silent[t] and not silent[t - 1]]
    self.assertEqual(entries, [31 * k - 1 for k in range(1, 20)])
    self.assertEqual([t for t in entries if pulse[t - 1] > 0],
                     [61, 92, 123, 154, 247, 278, 309, 340, 433, 464, 495, 526])
    starts = pulses(trace)
    self.assertEqual(starts[:25], [1 + 8 * k for k in range(23)] + [187, 195])
    self.assertEqual(len(starts), 75)
    self.assertGreaterEqual(min(b - a for a, b in zip(starts, starts[1:])), 8)
    self.assertFalse(any(silent[t] for t in starts))
    self.assertLessEqual(max_in_window(changes(trace)), 16)

  def test_f18_held_across_entry(self):
    def inputs(t):
      if t <= 9:
        return flat(100, flags=BEAT, notes=[(60, 100)])
      return flat(100, flags=BEAT if t == 300 else 0)
    runs = self.modes(inputs, ps=(0, 2, 3))
    self.assertEqual(changes(runs[0]), [])
    self.assertEqual(changes(runs[2]), [300])
    self.assertEqual(pulses(runs[3]), [300])

  def test_f19_hid_digital_only(self):
    # The DMG pad has no shoulder: the receiver's energy is fed as values.
    def inputs(t):
      e = 200 if 320 <= t <= 379 else 64
      return frame_in(e, (e,) * 3, flags=BEAT if t >= 50 and (t - 50) % 60 == 0 else 0)
    runs = self.modes(inputs, ps=(0, 1, 2, 3))
    self.assertEqual(changes(runs[0]), [])
    spectral = runs[1]
    self.assertEqual(spectral[0][5:7], (1, 1))
    self.assertEqual([(t, spectral[t][5:7]) for t in changes(spectral)],
                     [(320, (2, 2)), (330, (3, 3)), (380, (2, 2)), (390, (1, 1))])
    cuts = [50, 110, 170, 230, 290, 320, 350, 410, 470, 530, 590]
    self.assertEqual(changes(runs[2]), cuts)
    gate = runs[3]
    self.assertEqual(pulses(gate), cuts)
    self.assertTrue(all(s[5] in (1, 3) for s in gate))

  def test_f20_level_saturation(self):
    runs = self.modes(lambda t: frame_in(100, (10,) * 3 if 200 <= t <= 399 else (250,) * 3), ps=(0, 1, 2, 3))
    self.assertEqual(changes(runs[0]), [])
    spectral = runs[1]
    self.assertEqual(spectral[0][5:8], (3, 3, 0))
    self.assertEqual([(t, spectral[t][5]) for t in changes(spectral)],
                     [(200, 2), (210, 1), (220, 0), (400, 1), (410, 2), (420, 3)])
    self.assertEqual(changes(runs[2]), [400])
    self.assertEqual(pulses(runs[3]), [400])
    self.assertTrue(all(s[5] in (1, 3) for s in runs[3]))

  def test_f21_scene_debounce(self):
    def scene(t):
      if t < 100 or 300 <= t <= 319 or t >= 450:
        return 6
      if t < 200:
        return 7 if t % 2 == 0 else 6
      return 7
    trace = self.rom.run(lambda t: {"in": flat(100), "scene": scene(t)}, wire=True)
    self.assertTrue(all(s is None for s in trace[:230]))
    self.assertTrue(all(s == (0, 0, V2, 0, 0, 3, 3, 0) for s in trace[230:480]))
    self.assertTrue(all(s is None for s in trace[480:]))

  def test_f22_deferred_exit_arming(self):
    def inputs(t):
      if t < 60:
        return {"in": flat(100), "scene": 7, "pal": 0}
      scene = 7 if t >= 300 else 5 if (t - 60) % 40 < 20 else 6
      return {"in": flat(100), "scene": scene, "pal": 3}
    trace = self.rom.run(inputs, wire=True)
    self.assertNotIn(None, trace)
    self.assertTrue(all(s[0] == 0 for s in trace[:330]))
    self.assertTrue(all(s[0] == 3 for s in trace[330:]))
    self.assertEqual(trace[330][5:7], (1, 1))

  def test_f24_relative_palette(self):
    def inputs(t):
      buttons = ("right",) if t in (10, 20, 30) else ("left",) if t == 130 else ()
      return {"buttons": buttons}
    trace = self.rom.run(inputs, pad=True, scene=7)
    self.assertTrue(all(s[0] == 0 for s in trace[:60]))
    self.assertTrue(all(s[0] == 2 for s in trace[60:]))


class Pad(Rom):
  """Default ROMs: one step per frame, buttons by step index."""

  def step(self):
    lo, hi = self.gb.read(STEP, 2)
    return lo | hi << 8

  def run(self, steps, buttons=lambda t: ()):
    """trace[t] is step t's trace; shown[t] and frames[t] are the shown
    trace (copied at VBlank with the registers) and the frame drawn with it."""
    self.gb.reset()  # the core may fill WRAM; Main clears it before step 0
    for _ in range(600):
      self.gb.run_frame()
      if self.step() == 1:
        break
    else:
      raise AssertionError("ROM never stepped")
    self.trace = [self.gb.read(TRACE, 9)]
    self.shown, self.frames = [], []
    while len(self.trace) < steps:
      # The core latches the pad a frame early, so the next step reads these.
      self.gb.run_frame(buttons(len(self.trace)))
      if self.step() != len(self.trace) + 1:
        raise AssertionError(f"step clock {self.step()} after {len(self.trace)} steps")
      self.trace.append(self.gb.read(TRACE, 9))
      self.shown.append(self.gb.read(SHOWN, 9))
      self.frames.append(self.gb.frame)


@unittest.skipUnless(TOOLS and CORE.exists(), "needs RGBDS and the SameBoy libretro core")
class GbColourFieldCaptures(unittest.TestCase):
  """Pixel checks of the data and operator ROMs at the spec boundaries."""

  @classmethod
  def setUpClass(cls):
    subprocess.run(["make", "-s", "-C", str(HOMEBREW), "FIXTURE=1"], check=True, capture_output=True)
    for role in ("data", "operator"):
      subprocess.run(["make", "-s", "-C", str(HOMEBREW), f"INPUT_ROLE={role}"], check=True, capture_output=True)
    CAPTURES.mkdir(parents=True, exist_ok=True)

  def start(self, role="data"):
    rom = Pad(HOMEBREW / ("chipviz-gb-operator.gb" if role == "operator" else "chipviz-gb.gb"), video=True)
    self.addCleanup(rom.close)
    return rom

  def assert_frame(self, rom, i, name):
    """Every pixel of frame i is its region's shade for the shown trace."""
    shown = rom.shown[i]
    self.assertEqual(shown[0], 1, name)
    trace = tuple(shown[1:])
    frame = rom.frames[i]
    self.assertEqual((frame.width, frame.height), (160, 144))
    shades = expected_shades(trace)
    for y in range(144):
      for x in range(160):
        self.assertEqual(frame.pixel(x, y), GREY[shades[region_of(trace[2], x, y)]], f"{name} pixel {x},{y}")
    (CAPTURES / f"{name}.png").write_bytes(frame.png())
    return trace

  def stable(self, rom, start, end, want):
    """First frame index whose shown trace satisfies want and equals the next."""
    return next(i for i in range(start, end) if rom.shown[i][0] == 1 and want(tuple(rom.shown[i][1:]))
                and rom.shown[i] == rom.shown[i + 1])

  def test_data_role_layouts_and_shades(self):
    rom = self.start()
    # Up selects scene 7 (debounced), B takes the pad source, Right twice
    # selects hard cuts (relative, so never armed), then a beat every 25
    # steps: act 0..3 is rotate, combo, rotate, next layout.
    press = {10: "up", 14: "b", 50: "right", 54: "right", **{t: "b" for t in range(100, 1100, 25)}}
    rom.run(1100, held(press))
    entry = next(t for t, s in enumerate(rom.trace) if s[0] == 1)
    self.assertIn(entry, (40, 41, 42))
    self.assertTrue(all(s[0] == 1 for s in rom.trace[entry:]))
    self.assertEqual({s[1] for s in rom.trace[100:]}, {2})
    for layout, name in ((V2, "v2"), (H2, "h2"), (V3, "v3"), (H3, "h3")):
      i = self.stable(rom, 100, 1000, lambda s: s[0] == 2 and s[2] == layout)
      self.assert_frame(rom, i, f"cuts-{name}")

  def test_data_role_spectral(self):
    rom = self.start()
    # Procedural input (no B or A) in spectral mode: Right once.
    rom.run(400, held({10: "up", 50: "right"}))
    levels = set()
    for i in range(100, 398):
      if rom.shown[i][1] == 1 and rom.shown[i] == rom.shown[i + 1] and rom.shown[i][6:8] not in levels:
        levels.add(rom.shown[i][6:8])
        self.assert_frame(rom, i, f"spectral-v2-{rom.shown[i][6]}{rom.shown[i][7]}")
    self.assertGreaterEqual(len(levels), 3)
    changes_at = [t for t in range(101, 400) if rom.trace[t] != rom.trace[t - 1]]
    self.assertGreaterEqual(min(b - a for a, b in zip(changes_at, changes_at[1:])), 10)

  def test_data_role_never_flashes(self):
    rom = self.start()
    press = {10: "up", 14: "b", 50: "right", 54: "right", 58: "right", **{t: "b" for t in range(100, 400, 9)}}
    rom.run(400, held(press))
    self.assertEqual({s[1] for s in rom.trace[45:] if s[0] == 1}, {0, 2})
    self.assertEqual(rom.trace[399][1], 2)

  def test_operator_warning_and_gate(self):
    rom = self.start("operator")
    press = {10: "up", 14: "b", 20: "right", 24: "right", 28: "right"}
    rom.run(80, held(press))
    i = next(i for i, s in enumerate(rom.shown) if s[0] == 2)
    self.assertLess(i, 40)
    (CAPTURES / "operator-warning.png").write_bytes(rom.frames[i + 2].png())
    rom = self.start("operator")
    beats = {t: "b" for t in range(100, 300, 20)}
    rom.run(300, held({10: "up", 14: "b", 20: "right", 24: "right", 28: "right", 40: "start", **beats}))
    self.assertEqual({s[1] for s in rom.trace[100:]}, {3})
    lit = self.stable(rom, 100, 298, lambda s: s[5] == 3)
    rest = self.stable(rom, 100, 298, lambda s: s[5] != 3)
    self.assertEqual(rom.shown[rest][6], 1)  # dim rest: pad energy HID_E_LO
    self.assert_frame(rom, lit, "operator-gate-lit")
    self.assert_frame(rom, rest, "operator-gate-rest-dim")
    starts = [t for t in range(101, 300) if rom.trace[t][6] == 3 and rom.trace[t - 1][6] != 3]
    self.assertGreaterEqual(min(b - a for a, b in zip(starts, starts[1:])), 8)

  def test_fixture_black_rests(self):
    # Silence (F0) on the fixture build: spectral black, and the flash/gate
    # dark rest, which needs pre-arming (unreachable from the data role).
    rom = Fixture(HOMEBREW / "chipviz-gb-fixture.gb", video=True)
    self.addCleanup(rom.close)
    for p, name in ((1, "spectral-silence-black"), (3, "gate-rest-dark-armed")):
      rom.run(lambda t: bytes(22), steps=40, p=p)
      rom.gb.run_frame()
      rom.gb.run_frame()
      rom.shown, rom.frames = [rom.gb.read(SHOWN, 9)], [rom.gb.frame]
      trace = self.assert_frame(rom, 0, name)
      self.assertEqual((trace[0], trace[5:]), (p, (0, 0, 0)))

  def test_operator_pin(self):
    rom = self.start("operator")
    # Select cycles the pin: auto, V2, H2, ...; pinning H2 changes the layout.
    rom.run(120, held({10: "up", 14: "b", 30: "select", 34: "select"}))
    self.assertEqual(rom.trace[119][3], H2)
    self.assert_frame(rom, self.stable(rom, 60, 118, lambda s: s[2] == H2), "operator-pin-h2")


def held(table):
  """Each press is held for two steps: the core latches the pad at a frame
  boundary, so a one-step press can land on either step."""
  return lambda t: (table[t],) if t in table else (table[t - 1],) if t - 1 in table else ()


if __name__ == "__main__":
  unittest.main()
