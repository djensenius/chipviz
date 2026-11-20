# Opt-in scene catalog v1

This is a specification for future implementations, not a claim that any
sender or receiver implements it. It applies to macOS, Pi, Game Boy, NES,
C64, GBA, Master System, Genesis, SNES and N64. Adaptations may look native,
but must preserve the identity and selectable variants below. No static
diagnostic counts as a family.

The legacy default remains [control-frame v0](control-frame-v0.md), including
its platform-specific scene interpretation, procedural/native scenes,
controls and input roles. Raw UDP and raw 33-byte-record `.cvz` remain legacy.
Identical v0 bytes cannot identify the intended catalog: for example, modern
v0 scene 6 wraps to SignalField, while catalog v1 scene 6 is Starfield. Never
infer opt-in from a scene, palette or reserved flag bit. A mode change requires
explicit configuration at both ends; do not silently fall back across modes.
Existing [ColourField](colour-field-v0.md),
[HID](usb-hid-transport-v0.md), and
[Joybus](n64-joybus-transport-v0.md) contracts remain in force.

## Catalog and variants

Catalog ID is `1`. In an explicitly opted-in record, the v0 payload's scene
byte is the **exact** family ID below, not a modulo index. Values above 13 are
invalid. Variant is separate metadata; it never consumes palette or flag
bits. The valid `(family, variant)` pairs are:

| ID | Family | Variants |
| --- | --- | --- |
| 0 | SignalField | 0 |
| 1 | SpectrumVolume | 0 |
| 2 | Pulse Tunnel | 0 existing tunnel; 1 hallway; 2 abstract moving lines |
| 3 | Lines | 0 |
| 4 | Static | 0 |
| 5 | Object | 0 |
| 6 | Starfield | 0 |
| 7 | ColourField | 0 |
| 8 | DMG Tile Scroll | 0 |
| 9 | NES Diamond Scroll | 0 |
| 10 | C64 PETSCII Meter Field | 0 |
| 11 | GBA Blockscape | 0 base blocks; 1 tunnel; 2 stripes; 3 star-like blocks |
| 12 | SNES Tile Meters | 0 |
| 13 | N64 Perspective Meters | 0 |

Variant 0 is the default for every family. The GBA star-like blocks are not
the astronomical Starfield; Lines is distinct from Pulse Tunnel's hallway.
Unknown or unavailable families/variants fail explicitly, never alias or
substitute another scene. Platforms must eventually provide all pairs; until
implemented they must report the missing pair. A request changes family and
variant together, only after validation. Variant configuration is independent
per family, not a ColourField layout or palette.

Existing scene next/previous controls in an explicitly extended receiver use
its ring of **available** families, in ascending global ID order, wrapping at
the ends. A family is available only if its configured variant is implemented,
validated and usable with its required inputs on that receiver. Its configured
variant starts at 0; do not silently substitute another variant. Skipping a
family is not evidence that it is implemented or selectable; coverage needs
independent evidence. Direct requests for unsupported or unavailable pairs,
or for families outside the session ring, fail explicitly without aliasing to
a ring member. An empty ring cannot enter extended mode; in a one-family ring
both directions leave selection unchanged.

Establish the exact ring before starting and keep it fixed for the session.
If availability or the configured variant changes in a way that changes the
ring, stop input, enter a non-flashing fallback and require explicit reset,
revalidation and confirmation of the new ring before resuming. Never silently
skip an unexpectedly unavailable member during a session. Relative senders
and operators must know and reconfirm the exact receiver-specific ring before
starting or resynchronizing. Passing over unavailable IDs is not an input for
those IDs and does not bypass scene-7 gates when entering or leaving 7.

Existing palette, layout, role, auto/demo/manual and operator controls retain
their meanings; select variants separately, through startup configuration
or the per-record digital variant field, without stealing those controls.
Selection of a family restores its configured variant; a validated digital
record explicitly selects its own available variant without changing that
configuration. Legacy mode keeps its own rings and defaults. Procedural/demo
sources must not select scene 7 or arm flash. If a target lacks a safe variant
configuration path, it must report the limitation rather than reinterpret an
existing control.

## Digital record and file

All integers are little-endian; signed integers are two's complement. The
record is exactly **145 bytes**, one whole UDP datagram per record. Byte
offsets are zero-based, inclusive:

| Bytes | Field | Requirement |
| --- | --- | --- |
| 0–3 | magic | ASCII `CVX1` |
| 4 | record version | `1` |
| 5 | catalog | `1` |
| 6 | variant | valid for payload scene |
| 7 | reserved | zero |
| 8–15 | sequence | `u64` |
| 16–23 | tick | `u64`, equal to sequence |
| 24–56 | v0 payload | exactly 33 bytes, with scene interpreted in catalog 1 |
| 57–88 | Starfield asset SHA-256 | see below |
| 89–96 | Starfield epoch | signed `i64` Unix milliseconds |
| 97–100 | latitude | signed `i32`, degrees × 10⁷, north positive |
| 101–104 | longitude | signed `i32`, degrees × 10⁷, east positive |
| 105–108 | elevation | signed `i32`, millimetres above WGS84 ellipsoid |
| 109–140 | provenance SHA-256 | see below |
| 141–144 | CRC-32 | bytes 0–140 |

CRC-32 is CRC-32/ISO-HDLC: reflected polynomial `0xEDB88320`, initial
`0xFFFFFFFF`, final XOR `0xFFFFFFFF` (check `123456789` = `0xCBF43926`).
CRC and hashes detect errors, not malicious traffic or authenticate a sender.
The embedded v0 frame must independently have magic `C7`, version 0, correct
XOR, clear reserved flag bits, note count at most 4 and active note numbers
at most 127. Its palette remains the unmodified v0 palette. Its `frame`
counter equals sequence modulo 65536. For non-Starfield records bytes 57–140
must all be zero.

An extended recording uses `.cvxe`, not `.cvz`. Its 20-byte header is ASCII
`CVXE` at 0–3, version `1` at 4, catalog `1` at 5, zero at 6–7, record count
`u64` at 8–15, then CRC-32 of bytes 0–15 at 16–19. It is followed by exactly
that many complete records, with no footer or trailing bytes. Count must be
1..1,000,000; calculate `20 + 145 * count` with checked arithmetic before
allocation. Header and all records, ordering, referenced Starfield inputs and
exact EOF must validate before playback has any effect. Records retain their
own catalog/variant metadata; no mixed catalogs or formats are permitted.

Updated digital readers in legacy mode accept only complete legacy input;
extended mode accepts only the specified magic, versions, sizes and values.
Check the full datagram length and truncation indication, using an adequately
sized receive buffer or an API exposing truncation; receiving only 33 bytes
cannot establish the datagram length. Reject unknown, malformed, truncated,
oversized, wrong-mode, bad-integrity or unavailable input before it affects
selection or safety state. Never strip the envelope and pass it to a legacy
reader. In particular, the current modern UDP receiver reads into a 33-byte
buffer and can truncate a larger datagram. The first byte `43`, not `C7`,
prevents that legacy modern parser from accepting the truncated prefix, but
does not make all old consumers safe. Some old `.cvz` converters check only
33-byte alignment; even a truncated foreign file can satisfy that check.
Use explicit routing and verified updated readers, not extensions or magic as
a universal safety guarantee. Legacy fixtures are not rewritten.

### Ordering and clock

A session begins only after explicit receiver reset/opt-in, at sequence and
tick 0; each following record increments both by exactly one, with no `u64`
wrap. Tick `n` denotes exactly `n/60` seconds after the session epoch. Each
applied record is one step. Offline processing may run without a wall clock
but retains this step time; real-time playback must use the inter-step spacing
below and must not skip, interpolate or burst steps to catch up. A recording
must be contiguous from zero. These counters do not identify sessions or
authenticate traffic; operators must isolate the route and flush stale packets
when restarting.

For live UDP, validate each complete record before queueing it. Queue at most
four waiting records in order, not counting a record already applied. The
first expected sequence is 0; thereafter it is one greater than the highest
sequence applied or queued. A valid duplicate or older sequence, including
one already queued, is rejected without changing the queue or taking a step.
An expected record may be queued only if capacity remains; a gap or overflow
invalidates the session. Do not reorder, drop, merge or interpolate records.
Queueing is not a ColourField input: no edge detection, stability counter,
selection or arming state advances until application.

Apply the oldest queued record only when at least **16,666,667 ns** of
monotonic wall time has elapsed since the most recent extended application or
ColourField step, including across scene, mode and session changes. The first
record may apply immediately only when that spacing already holds or there
was no previous step. This non-burst pacing ensures no more than 60 steps in
any one-second window, including while scene 7 is active; a delayed scheduler
must not catch up by applying multiple records together. Hosts must also obey
the existing ColourField send-rate cap. Unlike the legacy over-rate resampling
path, these extended records enter the ColourField state machine only at paced
application; overflow fails closed instead of silently resampling away a
record. All ColourField decode, edge, entry, debounce, arming and budget rules
still apply to those ordered steps.

A gap, overflow, malformed or otherwise invalid record, disconnect, reset or
mode change invalidates the session: discard the queue, stop applying input,
clear pending requests and arming state, and immediately present steady black
until explicit reset and a valid selection passes the applicable gates.
Never count rejected or held frames toward stability or synthesize beat,
palette or arm inputs. The blackout is a single safety cut, not a synthetic
scene/palette input; it must not wait for scene debounce while flashing
continues. Clear stability windows and the latch, but retain scene-7 transition
and change-budget history across restart. Account for blackout from scene 7
as an exit when enforcing re-entry and the budget, without inventing a scene
input. Re-entry must satisfy normal debounce and budget, not the power-on
entry exemption; do not count blackout time as input steps.

Live UDP has no inherent disconnect notification. Use a monotonic real-time
deadline of **500 ms** after explicit opt-in while awaiting sequence 0, or
after the last validated record **applied** thereafter. Queueing, duplicates
and older records never refresh it. At or after the deadline, before processing
or applying anything else, invalidate the session as above, even if a record
is waiting. An independent timer must enforce it when no packets arrive or
the application scheduler stalls. No later packet can resume a timed-out
session. This permits 30 nominal 60 Hz intervals without applied progress
while bounding stale armed output; it does not identify a sender or prove a
disconnect. The timeout does not apply to offline processing or file playback,
which remain deterministic on their tick clock.

### Starfield inputs and provenance

Starfield requires user-supplied local catalog bytes and a local provenance
document whose SHA-256 digests exactly match bytes 57–88 and 109–140. Neither
digest may be all zero. The document is UTF-8, exactly seven LF-terminated
lines: `chipviz-star-provenance-v1`, then `source=`, `license=`,
`conversion=`, `coordinate-epoch=`, `model=`, and `implementation=` in that
order. Each key has a nonempty value without control characters. Values
identify the catalog source and licence, conversion tool/version/options (or
`none`), coordinate epoch, astrometry/time model/version and implementation
identifier/version respectively. `implementation` is `<id>@<major>.<minor>.<patch>`:
`id` is a publisher-controlled reverse-domain identifier, at most 64 ASCII
characters, of two or more dot-separated nonempty lowercase alphanumeric or
hyphen components; each version component is decimal `0` or starts with
`1..9` and is at most 9 digits. The exact identifier and version must identify
the renderer implementation used and must be explicitly supported by the
receiver; missing, ambiguous or unknown identifiers or versions are rejected.
Do not embed private catalog bytes or the provenance document in the recording.
Verify both hashes and the document before use; absent, malformed, unauthorized,
unsupported or mismatched inputs make Starfield unavailable, not a different
sky. The user is responsible for rights to supplied data. No HYG data or
HYG-derived media is bundled or published; SDK-derived assets/media remain
subject to their own privacy gates.

Latitude must be within ±900000000, longitude within ±1800000000, elevation
within -12000000..100000000 mm, and epoch within
946684800000..4102444799999 Unix milliseconds (2000 through 2099 UTC).
Unix time ignores leap seconds; reject unsupported dates/models and ticks
whose computed sky time is outside the stated epoch range. The sky time
for tick `n` is epoch + `n/60` seconds, using exact rational time, not the
wall clock. Reproducibility requires the same asset bytes, provenance/model,
location, epoch, ticks and implementation version. Exact cross-platform
astrometric math, CSV validation, conversion and numeric tolerances are a
dependent Starfield contract; until agreed and implemented, a receiver must
report Starfield unavailable rather than claim identical or accurate output.
The metadata of all Starfield records in a session must match, even if other
families occur between them; changing it requires a new session.

## Reduced inputs and safety

Absolute digital frames use their validated catalog scene for ColourField
decoding. Keep the existing per-transport host flash guard, warning,
scene-7 entry/exit debounce, palette stability, arming window and change
budget. Only a validated absolute scene 7 and palette in the same report can
contribute to absolute wire arming. Rejected metadata/frames are not inputs
and cannot arm; malformed input requires restart as above, while queued,
duplicate and old records do not refresh the live deadline. Relative palette
pulses in the `data` role remain unarmed, including transit through 3/7. Do not
weaken operator warning/confirmation or allow a HID data feed into an operator
build. Scene 7 variant 0 retains all its existing palette and layout rules.

HID v0's eight absolute scene keys cannot identify all 14 IDs. Joybus v1's
marker identifies neither this catalog nor a compatible ROM; old N64 ROMs can
ignore it. Do not send extended Joybus traffic to an old or unverified N64
receiver. A future mapping must be separately specified and validated before
use; the existing HID and Joybus encoders are not extended encoders.

For relative pad paths, no pulse carries catalog identity or proves delivery.
Before sending, independently confirm the exact receiver build, extended
catalog and receiver-specific available-family ring, `data` role, actual
scene, palette, variant and **unarmed** state. Require a receiver-polled
release preamble of at least 60 consecutive steps with all controls released,
then deliver every press and release in order at the receiver's required rate,
respecting debounce and budget. Use
one axis per poll, with at least 59 released polls after each press before
the next press, unless a future validated mapping requires a longer interval.
Intermediate ring positions are visible, not atomic. After loss, reset,
disconnect, ring or availability change, replay from an unknown point or
external input, stop; independently reconfirm the exact ring, re-establish all
state and repeat the polled release before resuming. A host
assertion is not detection, feedback or a handshake. In particular, do not
assume an unarmed state merely from a palette number. Existing
[GBA HID preconditions](usb-hid-transport-v0.md#gba-gamepad-only-json-conversion)
remain applicable to its legacy ring and do not establish an extended ring.

## Conformance vectors

Numbers below are hexadecimal bytes unless stated otherwise. CRCs use the
algorithm above. A valid example has sequence/tick 0, variant 0, scene 7,
palette 2, all other payload fields zero. Its 33-byte v0 payload is:

```text
c7 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 07 02 00 00 00 00 00 00 00 00 00 00 c2
```

Its record is `43 56 58 31 01 01 00 00`, sixteen zero bytes, the payload
above, 84 zero bytes, then CRC `c3 cc bb 8b` (145 bytes). A one-record file
prepends header `43 56 58 45 01 01 00 00 01 00 00 00 00 00 00 00 bf 5d 1f 65`
and is 165 bytes.

For each mutation below, recompute the inner v0 XOR after changing any payload
byte, except when intentionally testing bad XOR. Then recompute the outer CRC,
except when intentionally testing bad outer CRC, so the intended validation
is exercised.

- Scene 6/variant 0 with zero Starfield metadata: reject, not SignalField.
  Reject a nonzero asset digest without the matching local asset, a provenance
  hash mismatch, malformed provenance, or latitude 900000001.
  Scene 7/variant 1, scene 14, catalog 2, version 2 or reserved byte 1:
  reject. Scene 2 variants 0, 1, 2 and scene 11 variants 0, 1, 2, 3 are
  distinct valid selections when implemented; their next variant is invalid.
- Corrupt inner XOR, set v0 reserved flag bit 5, or set note count to 5:
  reject even with a correct outer CRC. A flipped outer CRC also rejects.
- Starting from the example, scene 2/variant 0 has inner XOR `c7` and outer
  CRC `52 55 2f d6`; scene 2/variant 2 has XOR `c7` and CRC `5f 1c 03 e5`.
  Both are accepted when implemented and available. Scene 2/variant 3 has
  XOR `c7` and CRC `f9 bb 2d 11`: reject for variant. Scene 14/variant 0
  has XOR `cb` and CRC `51 75 ce b2`: reject for scene. Setting flags to
  `20` has XOR `e2` and CRC `e9 57 aa 5a`: reject for reserved flags.
  Setting note count to 5 has XOR `c7` and CRC `b5 99 9f 81`: reject for
  note count. For an intentionally bad XOR, change checksum `c2` to `c3`
  without changing any other payload byte; outer CRC `42 10 3a 80` is correct
  but the inner XOR is not. These CRCs cover otherwise unchanged example records.
- Reject record lengths 0, 33, 144 and 146; reject a truncated datagram even
  when its visible 145-byte prefix is otherwise valid. Reject the example
  file with a missing byte, extra byte, wrong count, bad header CRC, or any
  invalid record, before playing its first record.
- After accepting sequence 0, duplicate 0 makes no step; sequence 2 causes
  desynchronization, not a fabricated sequence 1. Sequence 1 after that
  cannot resume without explicit reset. Sequence 65536 has payload frame 0.
- With available ring `0, 2, 7, 11`, next from 0 is 2, previous from 0 is 11,
  and next from 2 requests 7 through the normal scene-7 debounce. Direct
  request 1 fails, not scene 2; 1 is not counted as implemented. If 2 becomes
  unavailable, stop and reconfirm the new ring before further pulses. An
  empty ring cannot start; `7` alone leaves next/previous unchanged. In ring
  `0, 11`, next from 0 requests 11, not scene 7.
- In live UDP, opt-in at 0 ms without an applied record times out at 500 ms.
  If sequence 0 is applied at 0 ms, repeated 0 at 499 ms does not refresh
  its deadline: at 500 ms clear pending/arming and show steady black, even
  without another datagram. Sequence 1 at 500 ms cannot revive the session.
  If sequence 1 is instead applied at 499 ms, the deadline is 999 ms; an
  older sequence 0 at 998 ms does not extend it.
  If scene 7 was active, a restart cannot use the power-on exemption to bypass
  its debounce or budget. Offline processing has no wall-clock timeout.
- If sequence 0 applies at 0 ns and sequences 1 and 2 arrive at 1 ms and
  2 ms, respectively, both wait; their earliest applications are 16,666,667
  ns and 33,333,334 ns. A duplicate queued sequence 1 changes neither the
  queue nor the deadline. No counter or onset advances on queueing. With
  sequence 0 applied at 0 ms, sequences 1..4 queued at 1..4 ms fill the
  queue; sequence 5 at 5 ms causes immediate blackout, not a dropped step.
  If sequence 1 is queued at 1 ms but the application scheduler stalls,
  the independent timer blackouts at 500 ms: that queued record never
  refreshed the deadline and cannot apply at the deadline.
- For a hypothetical receiver supporting `org.example.renderer@1.2.3`, the
  exact UTF-8 bytes below, including the final LF, have SHA-256
  `e755ea1e5fc421dcf034e15a8ded19fbda696a1b39f49b2f54e9c2c9589d4bb3`
  and pass provenance field validation when the listed model is supported.
  A missing line, malformed version or unknown ID/version fails even when the
  supplied provenance hash is correctly recomputed.

  ```text
  chipviz-star-provenance-v1
  source=synthetic
  license=example
  conversion=none
  coordinate-epoch=J2000
  model=synthetic-v1
  implementation=org.example.renderer@1.2.3
  ```
- A legacy raw scene 6 stays legacy, even if its other bytes match the payload
  of an extended scene 6. A `CVX1` prefix truncated to 33 bytes is not a v0
  frame; an alignment-only converter may nevertheless accept an aligned file.
  The complete 165-byte example is itself divisible by 33, as is its
  132-byte truncated prefix.
- A validated scene 7 palette 3 without host flash opt-in is sent as palette
  2 with corrected checksums. With opt-in and warning, it still needs the
  existing absolute 30-step in-window arming and change budget. A rejected
  record, a relative transit through 3, or a stale held frame cannot complete
  that window.

## Dependency order

Implement and test strict digital codecs, mode routing and legacy fixture
preservation first. Specify and validate Starfield's model and private asset
pipeline separately. Then implement the catalog and any new reduced-transport
mapping per platform, including their safety state machines and controls.
Physical hardware, performance and flash-safety validation remain separate
from this specification; no implementation or hardware coverage is implied.
