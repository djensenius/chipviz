"""Deterministic Mesen (libretro) checks for the NES colour field (scene 7).

Each scenario drives controller 1 per step, reads the trace tuple from CPU RAM
(addresses from the ld65 label file) and checks frame-buffer captures for
layout boundaries and per-region colours. Expected colours and combos come
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

  def __init__(self, role):
    rom = HOMEBREW / ("chipviz-nes-operator.nes" if role == "operator" else "chipviz-nes.nes")
    self.labels = {}
    for line in (HOMEBREW / "build" / role / "labels.txt").read_text().splitlines():
      _, address, name = line.split()
      self.labels[name.lstrip(".")] = int(address, 16)
    self.nes = nlc.NesCore(rom, CORE)
    self.states = []  # RAM after each step, by step index

  def var(self, ram, name, size=1):
    address = self.labels[name]
    return ram[address] if size == 1 else bytes(ram[address:address + size])

  def run(self, steps, script, captures=()):
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
        if step in captures:
          shots[step] = self.nes.frame
    return shots

  def trace(self, step):
    return tuple(self.var(self.states[step], "Mode", 8))

  def close(self):
    self.nes.close()


def presses(table):
  """Script from {step: buttons}: each press lasts one step."""
  return lambda step: table.get(step, ())


@unittest.skipUnless(TOOLS and CORE.exists(), "needs cc65 and the Mesen libretro core")
class NesColourFieldTests(unittest.TestCase):
  rgb_of = {}

  @classmethod
  def setUpClass(cls):
    for role in ("data", "operator"):
      subprocess.run(["make", "-s", "-C", str(HOMEBREW), f"INPUT_ROLE={role}"], check=True, capture_output=True)
    CAPTURES.mkdir(parents=True, exist_ok=True)

  def start(self, role="data"):
    run = Run(role)
    self.addCleanup(run.close)
    return run

  def assert_capture(self, run, step, frame, name):
    """Every pixel is its region's expected colour, at the spec boundaries."""
    trace = run.trace(step)
    expected = expected_indices(trace)
    palette = run.var(run.states[step], "PalBuf", 16)
    self.assertEqual([palette[4 * i + 1] for i in range(len(expected))], expected, name)
    self.assertNotIn(0x0D, palette)
    layout = trace[2]
    colours = [frame.pixel(*next((x, y) for y in range(0, 240, 8) for x in range(0, 256, 8)
                                 if region_of(layout, x, y) == i)) for i in range(len(expected))]
    for index, rgb in zip(expected, colours):
      self.assertEqual(self.rgb_of.setdefault(index, rgb), rgb, f"{name}: NES ${index:02X}")
    for y in range(240):
      for x in range(256):
        self.assertEqual(frame.pixel(x, y), colours[region_of(layout, x, y)], f"{name} pixel {x},{y}")
    (CAPTURES / f"{name}.png").write_bytes(frame.png())

  def test_scene_debounce(self):
    run = self.start()
    toggles = {step: ("up",) for step in range(1, 200, 10)}  # ring alternates every 10 steps
    toggles.update({0: ("b",), 230: ("up",), 300: ("down",)})
    run.run(400, presses(toggles))
    shown = [run.var(ram, "Shown7") for ram in run.states]
    self.assertEqual(shown[:259], [0] * 259)  # jitter never enters
    self.assertEqual(shown.index(1), 259)  # 230 is step 1; change at the end of 259
    self.assertEqual(run.var(run.states[260], "ShowMode"), 1)  # entry step
    self.assertEqual(run.trace(260), (0, 0, 0, 0, 0, 3, 3, 0))  # held V2, combo dom = bass
    self.assertEqual(shown[260:329], [1] * 69)
    self.assertEqual(shown[329:], [0] * (400 - 329))  # 300 is step 1; exit at the end of 329
    self.assertEqual(run.var(run.states[329], "ShowMode"), 0)

  def test_relative_palette_never_arms(self):
    # F24 on the NES pad ring, from the entry step E.
    run = self.start()
    table = {0: ("b",), 1: ("up",)}
    table.update({ENTRY + 10: ("right",), ENTRY + 20: ("right",), ENTRY + 30: ("right",), ENTRY + 130: ("left",)})
    run.run(ENTRY + 300, presses(table))
    modes = [run.trace(step)[0] for step in range(ENTRY, ENTRY + 300)]
    self.assertEqual(modes[:59], [0] * 59)
    self.assertEqual(modes[59:], [2] * (300 - 59))  # applied at the end of E+59
    self.assertEqual(run.var(run.states[ENTRY + 200], "PEff"), 3)  # the 2 at E+159 is cleared
    self.assertFalse(any(run.var(ram, "Latch") for ram in run.states))

  def test_scene_window_restarts_palette_count(self):
    # F22-style: the ring leaves 7 for 20 steps while the debounce keeps 7.
    run = self.start()
    table = {0: ("b",), 1: ("up",), ENTRY + 1: ("right",), ENTRY + 3: ("right",), ENTRY + 5: ("right",),
             ENTRY + 10: ("down",), ENTRY + 30: ("up",)}
    run.run(ENTRY + 100, presses(table))
    self.assertTrue(all(run.var(ram, "Shown7") for ram in run.states[ENTRY:]))
    modes = [run.trace(step)[0] for step in range(ENTRY, ENTRY + 100)]
    self.assertEqual(modes.index(2), 59)  # E+30 is step 1 of a clean window
    self.assertEqual(run.var(run.states[ENTRY + 99], "Latch"), 0)

  def cut_run(self, palette, name):
    """Hard cuts on pad beats: act 0 rotate, 1 combo, 2 rotate, 3 layout."""
    run = self.start()
    table = {0: ("b",), 1: ("up",)}
    table.update({ENTRY + 1 + 2 * k: ("right",) for k in range(palette)})
    applied = ENTRY + 2 * palette + 28  # 30th stable step of the ring value
    beats = [applied + 14] + [applied + 21 + 20 * k for k in range(17)] + [applied + 30]
    table.update({step: ("b",) for step in beats})
    end = applied + 21 + 20 * 17 + 10
    captures = {applied + 21 + 20 * k + 5 for k in range(17)}
    shots = run.run(end, presses(table), captures)
    self.assertEqual(run.trace(applied)[0:2], (2, palette >> 2))
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
    self.assertEqual(len(set(self.rgb_of.values())), len(self.rgb_of))  # distinct indices, distinct RGB

  def test_spectral_level_budget(self):
    run = self.start()  # procedural source: no B or A
    run.run(ENTRY + 600, presses({1: ("up",), ENTRY + 1: ("right",)}))
    self.assertEqual(run.trace(ENTRY + 31)[0], 1)
    traces = [run.trace(step) for step in range(ENTRY + 31, ENTRY + 600)]
    changes = [i for i in range(1, len(traces)) if traces[i] != traces[i - 1]]
    self.assertGreater(len(changes), 5)
    self.assertTrue(all(b - a >= 10 for a, b in zip(changes, changes[1:])), changes)  # LEVEL_OPP

  def test_data_role_never_flashes(self):
    run = self.start()
    table = {step: ("b",) for step in range(0, 900, 3)}
    table.update({1: ("up",), 50: ("right",), 52: ("right",), 54: ("right",)})  # 3, held 300 steps
    table.update({352: ("right",), 356: ("right",), 358: ("right",), 362: ("right",)})  # 7
    run.run(900, presses(table))
    self.assertEqual((run.var(run.states[300], "PalRing"), run.var(run.states[899], "PalRing")), (3, 7))
    self.assertFalse(any(run.var(ram, "Latch") or run.var(ram, "Mode") == 3 for ram in run.states))
    traces = [run.trace(step) for step in range(ENTRY + 1, 900)]
    changes = [i for i in range(1, len(traces)) if traces[i] != traces[i - 1]]
    self.assertTrue(all(sum(c <= x < c + 60 for x in changes) <= 3 for c in changes), changes)

  def test_operator_warning_arming_and_gate(self):
    run = self.start("operator")
    e = 2  # operator scene inputs are exempt: Up on step 1, entry on step 2
    table = {0: ("b",), 1: ("up",), e + 1: ("right",), e + 3: ("right",), e + 30: ("right",), e + 40: ("start",)}
    table.update({step: ("b",) for step in range(e + 80, e + 200, 2)})
    table.update({e + 200: ("right",)})  # 4: held, family B, disarms
    table.update({e + 240: ("right",), e + 242: ("right",), e + 244: ("right",), e + 250: ("left",)})
    shots = run.run(e + 300, presses(table), {e + 35})
    self.assertEqual(run.var(run.states[e], "ShowMode"), 1)
    self.assertEqual(run.trace(e + 20)[0], 2)  # the 1 then 2 requests: budget from entry
    self.assertEqual(run.var(run.states[e + 30], "ShowMode"), 2)  # unarmed 3 shows the warning
    warning = shots[e + 35]
    (CAPTURES / "operator-warning.png").write_bytes(warning.png())
    self.assertGreater(len(warning.colors(48, 0, 208, 112)), 1)  # text
    self.assertEqual(len(warning.colors(0, 112, 256, 240)), 1)  # rest is black
    self.assertEqual(run.var(run.states[e + 40], "ShowMode"), 1)
    armed = next(s for s in range(e + 40, e + 80) if run.trace(s)[0] == 3)
    self.assertEqual(run.var(run.states[armed], "Latch"), 1)
    starts = [s for s in range(e + 80, e + 200)
              if run.var(run.states[s], "Pulse") == 4 and run.var(run.states[s - 1], "Pulse") != 4]
    self.assertGreater(len(starts), 5)
    self.assertTrue(all(b - a >= 8 for a, b in zip(starts, starts[1:])), starts)
    lit = [run.trace(s)[5:7] for s in range(starts[0], starts[0] + 4)]
    self.assertEqual(lit, [(3, 3)] * 4)
    self.assertEqual(run.trace(e + 239)[0:2], (0, 1))
    self.assertEqual(run.var(run.states[e + 239], "Latch"), 0)
    self.assertEqual(run.var(run.states[e + 246], "ShowMode"), 2)  # unarmed 7
    self.assertEqual(run.trace(e + 299)[0:2], (2, 1))  # cancelled: mode 2, family B
    self.assertEqual(run.var(run.states[e + 299], "Latch"), 0)

  def test_operator_pin(self):
    run = self.start("operator")
    table = {0: ("b",), 1: ("up",), 10: ("select",), 12: ("select",), 14: ("select",), 16: ("select",)}
    table.update({step: ("b",) for step in range(40, 400, 20)})
    table.update({3: ("right",), 5: ("right",)})  # hard cuts
    shots = run.run(400, presses(table), {390})
    layouts = {run.trace(step)[2] for step in range(50, 400)}
    self.assertEqual(layouts, {3})  # pinned H3; action 3 rotates instead
    self.assertEqual(run.var(run.states[399], "Pin"), 3)
    self.assert_capture(run, 390, shots[390], "operator-pinned-H3")


if __name__ == "__main__":
  unittest.main()
