"""Inputs and normative expectations for the SMS test-only mailbox adapter.

These are not a Z80 execution harness. Each row is (scene, relative palette,
16-byte detector input) as accepted by detector_fixture.asm.
"""


def frame(energy, bands, spectrum=(0,) * 8, flags=0, note=None):
  if note is None:
    return bytes((energy, *bands, *spectrum, flags, 0, 0, 0))
  return bytes((energy, *bands, *spectrum, flags, 1, *note))


def f18(step):
  held = step < 10
  flags = int(held or step == 300)
  note = (60, 100) if held else None
  return 7, 2, frame(100, (100,) * 3, (100,) * 8, flags, note)


def f19(step):
  energy = 200 if 320 <= step < 380 else 64
  beat = int(step >= 50 and (step - 50) % 60 == 0)
  return 7, 1, frame(energy, (energy,) * 3, flags=beat)


def f20(step):
  value = 250 if step < 200 or step >= 400 else 10
  return 7, 1, frame(100, (value,) * 3)


def f21(step):
  if step < 100:
    scene = 6
  elif step < 200:
    scene = 7 if step % 2 == 0 else 6
  elif step < 300:
    scene = 7
  elif step < 320:
    scene = 6
  elif step < 450:
    scene = 7
  else:
    scene = 6
  return scene, 0, frame(100, (100,) * 3, (100,) * 8)


def f22(step):
  if step < 60 or step >= 300:
    scene = 7
  else:
    scene = 5 if ((step - 60) // 20) % 2 == 0 else 6
  palette = 0 if step < 60 else 3
  return scene, palette, frame(100, (100,) * 3, (100,) * 8)


def f24(step):
  palette = 0 if step < 10 else 1 if step < 20 else 2 if step < 30 else 3 if step < 130 else 2
  return 7, palette, frame(64, (64,) * 3)


def f19_expected_levels(step):
  level = 1 if step < 320 or step >= 390 else 2 if step < 330 or step >= 380 else 3
  return level, level, 0


def f20_expected_levels(step):
  if step < 200 or step >= 420:
    level = 3
  elif step < 210 or 410 <= step < 420:
    level = 2
  elif step < 220 or 400 <= step < 410:
    level = 1
  else:
    level = 0
  return level, level, 0


def f21_expected_shown(step):
  return 230 <= step < 480


def f24_expected_mode(step):
  return 0 if step < 60 else 2


def f22_expected_mode_no_flash(step):
  return 0 if step < 330 else 2
