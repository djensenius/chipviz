"""Validate the SMS target-mailbox fixture payloads and normative boundaries."""

import unittest

import sms_detector_fixtures as fixtures


class SmsDetectorFixtureTests(unittest.TestCase):
  def test_payload_shapes_and_ranges(self):
    for scenario in (fixtures.f18, fixtures.f19, fixtures.f20, fixtures.f21, fixtures.f22, fixtures.f24):
      for step in range(600):
        scene, palette, payload = scenario(step)
        self.assertIn(scene, (5, 6, 7))
        self.assertIn(palette, range(8))
        self.assertEqual(len(payload), 16)
        self.assertLessEqual(payload[13], 1)

  def test_f18_entry_held_and_release(self):
    self.assertTrue(all(fixtures.f18(step)[:2] == (7, 2) for step in range(600)))
    self.assertEqual(fixtures.f18(0)[2][12:], bytes((1, 1, 60, 100)))
    self.assertEqual(fixtures.f18(9)[2][12:], bytes((1, 1, 60, 100)))
    self.assertEqual(fixtures.f18(10)[2][12:], bytes((0, 0, 0, 0)))
    self.assertEqual(fixtures.f18(300)[2][12:], bytes((1, 0, 0, 0)))

  def test_f19_and_f20_expected_level_boundaries(self):
    self.assertEqual([fixtures.f19_expected_levels(s)[0] for s in (319, 320, 329, 330, 379, 380, 389, 390)],
                     [1, 2, 2, 3, 3, 2, 2, 1])
    self.assertEqual([fixtures.f20_expected_levels(s)[0] for s in (199, 200, 210, 220, 399, 400, 410, 420)],
                     [3, 2, 1, 0, 0, 1, 2, 3])

  def test_f21_scene_debounce_boundaries(self):
    self.assertEqual([fixtures.f21(s)[0] for s in (199, 200, 299, 300, 319, 320, 449, 450)],
                     [6, 7, 7, 6, 6, 7, 7, 6])
    self.assertEqual([fixtures.f21_expected_shown(s) for s in (229, 230, 479, 480)],
                     [False, True, True, False])

  def test_f24_relative_ring_never_requests_flash(self):
    self.assertEqual([fixtures.f24(s)[1] for s in (9, 10, 19, 20, 29, 30, 129, 130)],
                     [0, 1, 1, 2, 2, 3, 3, 2])
    self.assertEqual([fixtures.f24_expected_mode(s) for s in (0, 59, 60, 159, 599)], [0, 0, 2, 2, 2])

  def test_f22_closed_window_and_no_flash(self):
    self.assertEqual([fixtures.f22(s)[0] for s in (59, 60, 79, 80, 299, 300)],
                     [7, 5, 5, 6, 6, 7])
    self.assertEqual([fixtures.f22_expected_mode_no_flash(s) for s in (59, 89, 299, 329, 330, 599)],
                     [0, 0, 0, 0, 2, 2])


if __name__ == "__main__":
  unittest.main()
