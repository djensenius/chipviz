; Test-only step adapter; include after the detector and safe mode modules.
; It does not poll ports, access the VDP or change the shipped seed.
;
; Initialise via sms_fixture_init with A = initial scene. Before each
; sms_fixture_step, write the 16-byte detector input layout from data_input.asm
; to $C0A0-$C0AF, raw scene to $C0B0, and relative palette ring to $C0B1.
; The call runs exactly one step. Output $C0C0 = scene-7 trace valid; $C0C1
; through $C0C8 = trace (mode, family, layout, combo, rot, three levels);
; $C0C9-$C0CF = T, N, B, X, D, E, silent. In a native scene outputs are zero.
; Scene and palette are fixture values, not an absolute flash-arming path.
; The detector input supports at most one note pair.

.define sms_fx_input   $c0a0
.define sms_fx_scene   $c0b0
.define sms_fx_palette $c0b1
.define sms_fx_shown   $c0c0
.define sms_fx_trace   $c0c1
.define sms_fx_events  $c0c9

sms_fixture_init:
  call sms_control_init
  jp sms_input_init

sms_fixture_step:
  ld a,(sms_fx_scene)
  ld (sms_scene_ring),a
  ld a,(sms_fx_palette)
  and 7
  ld (sms_palette_ring),a
  ld hl,sms_fx_input
  ld de,sms_input_energy
  ld bc,16
  ldir
  call sms_begin_step
  ld a,(sms_visible)
  ld (sms_fx_shown),a
  or a
  jr nz,sms_fixture_field
  ld hl,sms_fx_trace
  ld b,15
  xor a
sms_fixture_clear:
  ld (hl),a
  inc hl
  djnz sms_fixture_clear
  jp sms_fixture_finish
sms_fixture_field:
  ld a,(sms_entry_frame)
  or a
  jr z,sms_fixture_later
  call sms_detector_entry
  call sms_field_scene_entry
  jr sms_fixture_snapshot
sms_fixture_later:
  call sms_detector_step
  call sms_field_step
sms_fixture_snapshot:
  call sms_field_make_trace
  ld hl,sms_field_trace
  ld de,sms_fx_trace
  ld bc,8
  ldir
  ld a,(sms_det_event_t)
  ld (sms_fx_events),a
  ld a,(sms_det_event_n)
  ld (sms_fx_events+1),a
  ld a,(sms_det_event_b)
  ld (sms_fx_events+2),a
  ld a,(sms_field_event_x)
  ld (sms_fx_events+3),a
  ld a,(sms_field_event_d)
  ld (sms_fx_events+4),a
  ld a,(sms_field_event_e)
  ld (sms_fx_events+5),a
  ld a,(sms_field_silent)
  ld (sms_fx_events+6),a
sms_fixture_finish:
  call sms_finish_step
  ld a,(sms_mode_changed)
  or a
  ret z
  call sms_field_mode_entry
  jp sms_ack_mode_changed
