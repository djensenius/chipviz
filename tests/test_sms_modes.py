"""Host fixtures for the SMS cut-action lookup, not Z80 execution."""

import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("sms_assets", ROOT / "cores/sms/homebrew/field_assets.py")
assets = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(assets)


def cuts(events, steps=600):
  """Model the spec's unpinned cut rule using the candidate action table."""
  actions = assets.cut_actions()
  act = layout = combo = rot = 0
  since_cut = 20
  accepted, layout_changes = [], []
  for step in range(steps):
    if step in events and since_cut >= 20:
      accepted.append(step)
      since_cut = 0
      action = actions[act]
      act = (act + 1) & 3
      regions = 3 if layout >= 2 else 2
      if action == 0:
        rot = (rot + 1) % regions
      elif action == 1:
        combo = (combo + 1) & 3
      elif action == 2:
        layout = (layout + 1) & 3
        rot %= 3 if layout >= 2 else 2
        layout_changes.append(step)
      else:
        raise AssertionError(f"unknown cut action {action}")
    since_cut = min(since_cut + 1, 20)
  return accepted, layout_changes, (layout, combo, rot)


class SmsCutModeTests(unittest.TestCase):
  def test_action_ring_state_after_each_cut(self):
    states = (
      (0, 0, 1), (0, 1, 1), (0, 1, 0), (1, 1, 0),
      (1, 1, 1), (1, 2, 1), (1, 2, 0), (2, 2, 0),
      (2, 2, 1), (2, 3, 1), (2, 3, 2), (3, 3, 2),
    )
    for count, expected in enumerate(states, 1):
      events = set(range(0, count * 20, 20))
      accepted, _, state = cuts(events, steps=count * 20)
      self.assertEqual(accepted, sorted(events))
      self.assertEqual(state, expected, count)

  def test_f3_drops_events_inside_cut_floor(self):
    accepted, _, _ = cuts({37, 101, 113, 250, 400, 523})
    self.assertEqual(accepted, [37, 101, 250, 400, 523])

  def test_f5_rhythmic_action_ring(self):
    accepted, layouts, _ = cuts(set(range(30, 600, 30)))
    self.assertEqual(len(accepted), 19)
    self.assertEqual(layouts, [120, 240, 360, 480])

  def test_f19_digital_event_sequence(self):
    events = {50, 110, 170, 230, 290, 320, 350, 410, 470, 530, 590}
    accepted, _, _ = cuts(events)
    self.assertEqual(accepted, sorted(events))
    self.assertEqual(len(accepted), 11)


if __name__ == "__main__":
  unittest.main()
