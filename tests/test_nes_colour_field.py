"""Deterministic Mesen (libretro) checks for the NES colour field (scene 7).

Each scenario drives controller 1 per step, reads the rendered trace (`View`)
from CPU RAM (addresses from the ld65 label file) and checks frame-buffer
captures for layout boundaries and per-region colours. Expected colours and combos come
from shared/specs/colour-field-v0.md, not from the ROM tables. The tests skip
when cc65 or the Mesen libretro core is not installed.
"""

import pathlib
import shutil
import subprocess
import sys
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import nes_libretro_capture as nlc


HOMEBREW = ROOT / "cores" / "nes" / "homebrew"
CAPTURES = ROOT / "build" / "nes-colour-field"
CORE = nlc.default_core()
TOOLS = shutil.which("ca65") and shutil.which("ld65") and shutil.which("make")

# Spec tables: NES level-3 and level-2 indices, and the combos.
BASE = {"R": 0x16, "Y": 0x28, "B": 0x12, "W": 0x30, "K": 0x0F, "G": 0x19, "C": 0x3C, "M": 0x14}
LEVEL2 = {"R": 0x06, "Y": 0x18, "B": 0x02, "W": 0x10, "K": 0x0F, "G": 0x09, "C": 0x2C, "M": 0x04}
COMBOS = {
  (0, 2): ["RB", "YB", "RY", "BW"],
  (0, 3): ["RYB", "RWB", "YKB", "RKY"],
  (1, 2): ["RC", "GM", "BY", "CM"],
  (1, 3): ["RGB", "CMY", "RKC", "BKY"],
}
# Mesen's default palette for the indices the field uses, fixed here so a wrong
# index cannot pass by agreeing with itself.
MESEN_RGB = {
  0x0F: (0, 0, 0),
  0x16: (181, 49, 32), 0x06: (108, 6, 0),
  0x28: (188, 190, 0), 0x18: (107, 109, 0),
  0x12: (66, 64, 255), 0x02: (20, 18, 167),
  0x30: (255, 254, 255), 0x10: (173, 173, 173),
  0x19: (56, 135, 0), 0x09: (11, 72, 0),
  0x3C: (181, 235, 242), 0x2C: (72, 205, 222),
  0x14: (160, 26, 204), 0x04: (92, 0, 126),
}
ENTRY = 31  # Up read on step 1 (0-based); 30 stable steps; entry step 31.


def regions(layout):
  return 3 if layout >= 2 else 2


def region_of(layout, x, y):
  """Spec geometry: g = 16, columns from the left, rows from the bottom."""
  n = regions(layout)
  size, pos = (256, x) if layout in (0, 2) else (240, 239 - y)
  bounds = [16 * ((i * (size // 16)) // n) for i in range(n)] + [size]
  return next(i for i in range(n) if bounds[i] <= pos < bounds[i + 1])


def expected_indices(trace):
  mode, family, layout, combo, rot, *levels = trace
  n = regions(layout)
  entries = COMBOS[(family, n)][combo]
  out = []
  for i in range(n):
    colour = entries[(i + rot) % n]
    out.append({3: BASE[colour], 2: LEVEL2[colour]}.get(levels[i], 0x0F))
  return out


class Run:
  """One ROM run, stepped once per frame with buttons chosen by step index."""

  ROMS = {"data": "chipviz-nes.nes", "operator": "chipviz-nes-operator.nes", "test": "chipviz-nes-test.nes"}

  def __init__(self, role):
    self.labels = {}
    for line in (HOMEBREW / "build" / role / "labels.txt").read_text().splitlines():
      _, address, name = line.split()
      self.labels[name.lstrip(".")] = int(address, 16)
    self.nes = nlc.NesCore(HOMEBREW / self.ROMS[role], CORE)
    self.states = []  # RAM after each step, by step index

  def var(self, ram, name, size=1):
    address = self.labels[name]
    return ram[address] if size == 1 else bytes(ram[address:address + size])

  def at(self, step, name, size=1):
    return self.var(self.states[step], name, size)

  def run(self, steps, script, captures=()):
    """Run steps 0..steps-1. captures[s] is the picture of step s's output.

    The NMI uploads the buffers step s built, then runs step s + 1, and the
    frame is drawn after it, so the frame of step s + 1 shows step s.
    """
    shots = {}
    while len(self.states) < steps:
      ram = self.nes.ram()
      step = self.var(ram, "StepLo") | self.var(ram, "StepHi") << 8
      if step != len(self.states):
        raise AssertionError(f"step clock {step} != {len(self.states)}: not one step per frame")
      self.nes.run_frame(script(step))
      ram = self.nes.ram()
      if (self.var(ram, "StepLo") | self.var(ram, "StepHi") << 8) == step + 1:
        self.states.append(ram)
        if step - 1 in captures:
          shots[step - 1] = self.nes.frame
    return shots

  def trace(self, step):
    """The trace rendered at step `step` (before its requests apply)."""
    return tuple(self.at(step, "View", 8))

  def output(self, step):
    return self.trace(step), self.at(step, "PalBuf", 16), self.at(step, "AttrShown")

  def changes(self, first, last):
    """Steps in first..last whose field output differs from the step before."""
    return [s for s in range(first + 1, last + 1) if self.output(s) != self.output(s - 1)]

  def close(self):
    self.nes.close()


def presses(table):
  """Script from {step: buttons}: each press lasts one step."""
  return lambda step: table.get(step, ())


@unittest.skipUnless(TOOLS and CORE.exists(), "needs cc65 and the Mesen libretro core")
class NesColourFieldTests(unittest.TestCase):

  @classmethod
  def setUpClass(cls):
    for role in Run.ROMS:
      subprocess.run(["make", "-s", "-C", str(HOMEBREW), f"INPUT_ROLE={role}"], check=True, capture_output=True)
    CAPTURES.mkdir(parents=True, exist_ok=True)

  def start(self, role="data"):
    run = Run(role)
    self.addCleanup(run.close)
    return run

  def assert_budget(self, run, first, last, limit=3):
    """At most `limit` visible changes in any 60 steps: 3 for cuts, pins and
    requests (CUT_MIN), 6 for spectral levels (LEVEL_OPP, 3 flashes/s)."""
    changes = run.changes(first, last)
    for c in changes:
      self.assertLessEqual(sum(c <= x < c + 60 for x in changes), limit, f"changes {changes}")
    return changes

  def assert_capture(self, run, step, frame, name):
    """Every pixel is its region's expected colour, at the spec boundaries."""
    self.assertEqual(run.at(step, "ShowMode"), 1, name)
    trace = run.trace(step)
    expected = expected_indices(trace)
    palette = run.at(step, "PalBuf", 16)
    self.assertEqual([palette[4 * i + 1] for i in range(len(expected))], expected, name)
    self.assertNotIn(0x0D, palette)
    layout = trace[2]
    colours = [MESEN_RGB[index] for index in expected]
    for y in range(240):
      for x in range(256):
        self.assertEqual(frame.pixel(x, y), colours[region_of(layout, x, y)], f"{name} pixel {x},{y}")
    (CAPTURES / f"{name}.png").write_bytes(frame.png())

  def test_reference_palette_is_injective(self):
    self.assertEqual(len(set(MESEN_RGB.values())), len(MESEN_RGB))
    self.assertEqual(set(MESEN_RGB), set(BASE.values()) | set(LEVEL2.values()))

  def test_scene_debounce(self):
    run = self.start()
    toggles = {step: ("up",) for step in range(1, 200, 10)}  # ring alternates every 10 steps
    toggles.update({0: ("b",), 230: ("up",), 300: ("down",)})
    shots = run.run(400, presses(toggles), {259, 260, 329, 330})
    shown = [run.var(ram, "Shown7") for ram in run.states]
    self.assertEqual(shown[:259], [0] * 259)  # jitter never enters
    self.assertEqual(shown.index(1), 259)  # 230 is step 1; change at the end of 259
    self.assertEqual([run.at(s, "ShowMode") for s in (259, 260)], [0, 1])  # entry step 260
    self.assertEqual(run.trace(260), (0, 0, 0, 0, 0, 3, 3, 0))  # held V2, combo dom = bass
    self.assertEqual(shown[260:329], [1] * 69)
    self.assertEqual(shown[329:], [0] * (400 - 329))  # 300 is step 1; decided at the end of 329
    self.assertEqual([run.at(s, "ShowMode") for s in (329, 330)], [1, 0])  # demo from 330
    self.assertEqual(shots[259].colors(0, 0, 256, 240) & {MESEN_RGB[0x16], MESEN_RGB[0x12]}, set())
    self.assert_capture(run, 260, shots[260], "debounce-entry")
    self.assert_capture(run, 329, shots[329], "debounce-last-field-step")
    self.assertNotEqual(shots[330].pixel(0, 0), shots[329].pixel(0, 0))

  def test_relative_palette_never_arms(self):
    # F24 on the NES pad ring, from the entry step E.
    run = self.start()
    table = {0: ("b",), 1: ("up",)}
    table.update({ENTRY + 10: ("right",), ENTRY + 20: ("right",), ENTRY + 30: ("right",), ENTRY + 130: ("left",)})
    run.run(ENTRY + 300, presses(table))
    modes = [run.trace(step)[0] for step in range(ENTRY, ENTRY + 300)]
    self.assertEqual(modes[:60], [0] * 60)  # applied at the end of E+59
    self.assertEqual(modes[60:], [2] * (300 - 60))  # visible from E+60
    self.assertEqual(run.at(ENTRY + 200, "PEff"), 3)  # the 2 at E+159 is cleared
    self.assertFalse(any(run.var(ram, "Latch") for ram in run.states))

  def test_scene_window_restarts_palette_count(self):
    # F22-style: the ring leaves 7 for 20 steps while the debounce keeps 7.
    run = self.start()
    table = {0: ("b",), 1: ("up",), ENTRY + 1: ("right",), ENTRY + 3: ("right",), ENTRY + 5: ("right",),
             ENTRY + 10: ("down",), ENTRY + 30: ("up",)}
    run.run(ENTRY + 100, presses(table))
    self.assertTrue(all(run.var(ram, "Shown7") for ram in run.states[ENTRY:]))
    modes = [run.trace(step)[0] for step in range(ENTRY, ENTRY + 100)]
    self.assertEqual(modes.index(2), 60)  # E+30 is step 1 of a clean window; applied at E+59
    self.assertEqual(run.at(ENTRY + 99, "Latch"), 0)

  def cut_run(self, palette, name):
    """Hard cuts on pad beats: act 0 rotate, 1 combo, 2 rotate, 3 layout."""
    run = self.start()
    table = {0: ("b",), 1: ("up",)}
    table.update({ENTRY + 1 + 2 * k: ("right",) for k in range(palette)})
    applied = ENTRY + 2 * palette + 28  # request applied at the end of this step
    beats = [applied + 14] + [applied + 21 + 20 * k for k in range(17)] + [applied + 30]
    table.update({step: ("b",) for step in beats})
    end = applied + 21 + 20 * 17 + 10
    captures = {applied + 21 + 20 * k + 5 for k in range(17)}
    shots = run.run(end, presses(table), captures)
    self.assertEqual(run.trace(applied)[0], 0)
    expected = (2, palette >> 2, 0, 0, 0, 3, 3, 0)
    for step in range(applied + 1, end):
      if step in beats[1:-1]:
        mode, family, layout, combo, rot, *_ = expected
        action = (beats.index(step) - 1) & 3
        if action in (0, 2):
          rot = (rot + 1) % regions(layout)
        elif action == 1:
          combo = (combo + 1) & 3
        else:
          layout = (layout + 1) & 3
          rot %= regions(layout)
        expected = (mode, family, layout, combo, rot, 3, 3, 3 if layout >= 2 else 0)
      self.assertEqual(run.trace(step), expected, f"step {step}")  # dropped beats change nothing
    for step, frame in sorted(shots.items()):
      trace = run.trace(step)
      self.assert_capture(run, step, frame, f"{name}-L{trace[2]}-c{trace[3]}-r{trace[4]}")
    return {run.trace(step)[2] for step in shots}

  def test_hard_cuts_family_a_layouts_and_colours(self):
    self.assertEqual(self.cut_run(2, "cuts-A"), {0, 1, 2, 3})

  def test_hard_cuts_family_b_layouts_and_colours(self):
    self.assertEqual(self.cut_run(6, "cuts-B"), {0, 1, 2, 3})

  def test_requests_interleaved_with_cuts_keep_budget(self):
    # Beats on every even step, palette ring 2 -> 6 -> 2 -> 6 ... (data ROM).
    run = self.start()
    table = {step: ("b",) for step in range(0, 700, 2)}
    table.update({1: ("up",), ENTRY + 1: ("right",), ENTRY + 3: ("right",)})
    for k, start in enumerate(range(ENTRY + 41, 640, 41)):
      table.update({start + 2 * j + 1: ("left" if k % 2 else "right",) for j in range(4)})
    run.run(700, presses(table))
    changes = self.assert_budget(run, ENTRY, 699)
    families = [run.trace(s)[1] for s in range(ENTRY, 700)]
    flips = sum(a != b for a, b in zip(families, families[1:]))
    self.assertGreaterEqual(flips, 4)  # family requests did apply between the cuts
    self.assertGreater(len(changes), 20)

  def spectral_run(self, palette, name):
    run = self.start()  # procedural source: no B or A
    table = {1: ("up",)}
    table.update({ENTRY + 1 + 2 * k: ("right",) for k in range(palette)})
    applied = ENTRY + 2 * palette + 28
    shots = run.run(ENTRY + 600, presses(table), set(range(applied + 1, ENTRY + 590, 3)))
    self.assertEqual(run.trace(applied + 1)[0:2], (1, palette >> 2))
    changes = self.assert_budget(run, ENTRY, ENTRY + 599, 6)
    self.assertGreater(len(changes), 5)
    traces = [run.trace(step) for step in range(applied + 1, ENTRY + 600)]
    steps = [i for i in range(1, len(traces)) if traces[i][5:] != traces[i - 1][5:]]
    self.assertTrue(all(b - a >= 10 for a, b in zip(steps, steps[1:])), steps)  # LEVEL_OPP
    level2 = [s for s in sorted(shots) if 2 in run.trace(s)[5:5 + regions(run.trace(s)[2])]]
    self.assertTrue(level2, "no level-2 frame captured")
    for step in level2[:2]:
      self.assert_capture(run, step, shots[step], f"{name}-{step}")
    return {i for s in level2 for i in expected_indices(run.trace(s))}

  def test_spectral_family_a_level_budget_and_level2_colours(self):
    self.assertTrue(self.spectral_run(1, "spectral-A") & {LEVEL2[c] for c in "RYBW"})

  def test_spectral_family_b_level_budget_and_level2_colours(self):
    self.assertTrue(self.spectral_run(5, "spectral-B") & {LEVEL2[c] for c in "RGBCMY"})

  def test_silence_blacks_out_spectral(self):
    # Test ROM: Select zeroes the pad energy; the bands stay 64 (level 1,
    # black on the NES), so only the silence blackout takes the levels to 0.
    run = self.start("test")
    table = {0: ("b",), 1: ("up",), ENTRY + 1: ("right",)}
    script = presses(table)
    silent = range(ENTRY + 100, ENTRY + 200)
    shots = run.run(ENTRY + 300, lambda s: ("select",) if s in silent else script(s), {ENTRY + 190})
    self.assertEqual(run.trace(ENTRY + 90), (1, 0, 0, 0, 0, 1, 1, 0))
    self.assertEqual(run.at(ENTRY + 99, "Silent"), 0)
    went = next(s for s in silent if run.at(s, "Silent"))
    dark = next(s for s in silent if run.trace(s)[5:] == (0, 0, 0))
    self.assertTrue(all(run.trace(s)[5:] == (1, 1, 0) for s in range(ENTRY + 90, went)))
    self.assertEqual(dark, went)  # blackout on the silence-entry step (lg >= LEVEL_OPP)
    self.assertTrue(all(run.trace(s)[5:] == (0, 0, 0) for s in range(dark, ENTRY + 200)))
    self.assertEqual(run.at(ENTRY + 200, "Silent"), 0)  # e >= SIL_EXIT ends silence at once
    self.assertEqual(run.trace(ENTRY + 299)[5:], (1, 1, 0))
    self.assert_budget(run, ENTRY, ENTRY + 299, 6)
    self.assert_capture(run, ENTRY + 190, shots[ENTRY + 190], "silence-black")

  def test_data_role_never_flashes(self):
    run = self.start()
    table = {step: ("b",) for step in range(0, 900, 3)}
    table.update({1: ("up",), 50: ("right",), 52: ("right",), 54: ("right",)})  # 3, held 300 steps
    table.update({352: ("right",), 356: ("right",), 358: ("right",), 362: ("right",)})  # 7
    run.run(900, presses(table))
    self.assertEqual((run.at(300, "PalRing"), run.at(899, "PalRing")), (3, 7))
    self.assertFalse(any(run.var(ram, "Latch") or run.var(ram, "Mode") == 3 for ram in run.states))
    self.assert_budget(run, ENTRY, 899)

  def test_operator_warning_arming_and_gate(self):
    run = self.start("operator")
    e = 2  # operator scene inputs are exempt: Up on step 1, entry on step 2
    table = {0: ("b",), 1: ("up",), e + 1: ("right",), e + 3: ("right",), e + 30: ("right",), e + 40: ("start",)}
    table.update({step: ("b",) for step in range(e + 80, e + 200, 2)})
    table.update({e + 200: ("right",)})  # 4: held, family B, disarms
    table.update({e + 240: ("right",), e + 242: ("right",), e + 244: ("right",), e + 250: ("left",)})
    shots = run.run(e + 300, presses(table), {e + 35})
    self.assertEqual(run.at(e, "ShowMode"), 1)
    self.assertEqual([run.trace(s)[0] for s in (e + 20, e + 21)], [0, 2])  # applied at the end of e+20
    self.assertEqual(run.at(e + 30, "ShowMode"), 2)  # unarmed 3 shows the warning
    warning = shots[e + 35]
    (CAPTURES / "operator-warning.png").write_bytes(warning.png())
    self.assertEqual(warning.colors(0, 0, 256, 16), {(0, 0, 0)})  # NTSC overscan stays black
    self.assertGreater(len(warning.colors(48, 16, 208, 88)), 1)  # text
    self.assertEqual(warning.colors(0, 88, 256, 240), {(0, 0, 0)})
    self.assertEqual(run.at(e + 40, "ShowMode"), 1)
    armed = next(s for s in range(e + 40, e + 80) if run.trace(s)[0] == 3)
    self.assertEqual(run.at(armed, "Latch"), 1)
    starts = [s for s in range(e + 80, e + 200) if run.at(s, "Pulse") == 4 and run.at(s - 1, "Pulse") != 4]
    self.assertGreater(len(starts), 5)
    self.assertTrue(all(b - a >= 8 for a, b in zip(starts, starts[1:])), starts)
    self.assertEqual([run.trace(s)[5:7] for s in range(starts[0], starts[0] + 4)], [(3, 3)] * 4)
    self.assertEqual(run.trace(e + 239)[0:2], (0, 1))
    self.assertEqual(run.at(e + 239, "Latch"), 0)
    self.assertEqual(run.at(e + 246, "ShowMode"), 2)  # unarmed 7
    self.assertEqual(run.trace(e + 299)[0:2], (2, 1))  # cancelled: mode 2, family B
    self.assertEqual(run.at(e + 299, "Latch"), 0)

  def test_operator_pin(self):
    run = self.start("operator")
    table = {0: ("b",), 1: ("up",), 10: ("select",), 12: ("select",), 14: ("select",), 16: ("select",)}
    table.update({step: ("b",) for step in range(40, 400, 20)})
    table.update({3: ("right",), 5: ("right",)})  # hard cuts
    shots = run.run(400, presses(table), {390})
    layouts = {run.trace(step)[2] for step in range(50, 400)}
    self.assertEqual(layouts, {3})  # pinned H3; action 3 rotates instead
    self.assertEqual(run.at(399, "Pin"), 3)
    self.assert_capture(run, 390, shots[390], "operator-pinned-H3")

  def test_operator_warning_after_layout_cut_reuploads_attributes(self):
    run = self.start("operator")
    table = {0: ("b",), 1: ("up",), 3: ("right",), 5: ("right",)}  # entry on 2, cuts from 23
    table.update({step: ("b",) for step in (48, 68, 88)})
    table.update({108: ("right", "b"), 120: ("a",)})  # layout cut and the warning on one step
    shots = run.run(140, presses(table), {130})
    self.assertEqual([run.trace(s)[2] for s in (107, 108)], [0, 1])
    self.assertEqual(run.at(108, "ShowMode"), 2)
    self.assertEqual(run.at(120, "ShowMode"), 1)  # cancelled
    self.assertEqual(run.trace(130)[0:3], (2, 0, 1))
    self.assert_capture(run, 130, shots[130], "operator-warning-cancel-H2")

  def test_operator_no_warning_on_exit_step(self):
    run = self.start("operator")
    table = {0: ("b",), 1: ("up",), 3: ("right",), 5: ("right",), 40: ("down", "right")}
    run.run(80, presses(table))
    self.assertEqual(run.at(40, "PalSel"), 3)
    self.assertEqual([run.at(s, "ShowMode") for s in (40, 41)], [1, 0])
    self.assertNotIn(2, [run.at(s, "ShowMode") for s in range(40, 80)])

  def test_operator_pin_on_entry_step(self):
    run = self.start("operator")
    run.run(10, presses({0: ("b",), 1: ("up",), 2: ("select",)}))  # entry on step 2
    self.assertEqual(run.at(2, "ShowMode"), 1)
    self.assertEqual((run.at(2, "Pin"), run.at(9, "Pin")), (0, 0))  # V2 pin kept


if __name__ == "__main__":
  unittest.main()
