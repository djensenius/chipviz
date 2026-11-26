; SMS gamepad-only data-role control module (WLA-Z80 syntax).
;
; This is included by the experimental candidate, not the SDK-free seed.
; The caller owns the VDP and reserves $C000-$C015 for this module. Each poll:
;   call sms_sample_pad (or pass active-high pad bits in A to sms_sample)
;   call sms_make_input (from data_input.asm)
;   call sms_begin_step
;   run/render the current scene; call sms_visible_change or sms_cut when
;     its mode rule makes a visible change or cut, respectively
;   call sms_finish_step
; The snapshot at sms_visible/sms_entry_frame and sms_peff is for the current
; step. Changes decided by finish_step are for the next step. Immediately
; after finish_step, a renderer must initialise a changed mode using the
; current step's inputs, then call sms_ack_mode_changed. This module never
; enables the display itself.
;
; Buttons from port $DC: up, down, left, right, 1, 2 in bits 0..5.
; sms_buttons holds the levels and sms_button_edges the rising edges in bits
; 4..5. This data-only interface has no absolute scene/palette or flash path.

.define sms_previous      $c000
.define sms_buttons       $c001
.define sms_button_edges  $c002
.define sms_scene_ring    $c003
.define sms_palette_ring  $c004
.define sms_deb_value     $c005
.define sms_deb_count     $c006
.define sms_since_scene   $c007
.define sms_shown         $c008
.define sms_entry_pending $c009
.define sms_visible       $c00a
.define sms_entry_frame   $c00b
.define sms_power_entry   $c00c
.define sms_peff          $c00d
.define sms_wire_value    $c00e
.define sms_wire_count    $c00f
.define sms_wire_used     $c010
.define sms_request       $c011
.define sms_request_valid $c012
.define sms_since_cut     $c013
.define sms_lg            $c014
.define sms_mode_changed  $c015

; A = initial scene. Pad builds use 0 or 7; fixture inputs may use other
; native scenes. Power-on scene 7 enters without debounce.
sms_control_init:
  ld b,a
  ld hl,sms_previous
  ld c,$16
  xor a
sms_init_clear:
  ld (hl),a
  inc hl
  dec c
  jr nz,sms_init_clear
  ld a,20
  ld (sms_since_scene),a
  ld a,b
  ld (sms_scene_ring),a
  ld (sms_deb_value),a
  cp 7
  ret nz
  ld a,1
  ld (sms_shown),a
  ld (sms_entry_pending),a
  ld (sms_power_entry),a
  ret

sms_sample_pad:
  in a,($dc)
  cpl
  and $3f
sms_sample:
  and $3f
  ld b,a
  ld (sms_buttons),a
  ld a,(sms_previous)
  cpl
  and b
  and $30
  ld (sms_button_edges),a
  ld a,(sms_previous)
  and $0f
  rlca
  rlca
  rlca
  rlca
  ld c,a
  ld a,b
  and $0f
  or c
  ld e,a
  ld d,0
  ld hl,sms_pad_edge_table
  add hl,de
  ld c,(hl)
  ld a,b
  ld (sms_previous),a
  bit 0,c
  jr z,sms_sample_palette
  ld a,(sms_scene_ring)
  xor 7
  ld (sms_scene_ring),a
sms_sample_palette:
  bit 1,c
  jr z,sms_sample_right
  ld a,(sms_palette_ring)
  dec a
  and 7
  ld (sms_palette_ring),a
sms_sample_right:
  bit 2,c
  ret z
  ld a,(sms_palette_ring)
  inc a
  and 7
  ld (sms_palette_ring),a
  ret

; Call after sampling and before running the step's mode rules.
sms_begin_step:
  ld a,(sms_shown)
  ld (sms_visible),a
  ld a,(sms_entry_pending)
  ld (sms_entry_frame),a
  or a
  ret z
  xor a
  ld (sms_entry_pending),a
  ld (sms_button_edges),a
  ld (sms_request_valid),a
  ld (sms_wire_used),a
  ld (sms_mode_changed),a
  ld a,(sms_palette_ring)
  ld (sms_wire_value),a
  call sms_unarmed_palette
  ld (sms_peff),a
  xor a
  ld b,a
  ld a,(sms_scene_ring)
  cp 7
  jr nz,sms_entry_wire_count
  inc b
sms_entry_wire_count:
  ld a,b
  ld (sms_wire_count),a
  xor a
  ld (sms_since_cut),a
  ld (sms_lg),a
  ld a,(sms_power_entry)
  or a
  jr z,sms_entry_done
  ld a,20
  ld (sms_since_cut),a
  ld a,10
  ld (sms_lg),a
sms_entry_done:
  xor a
  ld (sms_power_entry),a
  ret

; A = palette ring value. Mode 3 is always unarmed, including 7 -> 6.
sms_unarmed_palette:
  ld b,a
  and 3
  cp 3
  ld a,b
  ret nz
  dec a
  ret

sms_visible_change:
  xor a
  ld (sms_lg),a
  ret
sms_ack_mode_changed:
  xor a
  ld (sms_mode_changed),a
  ret
sms_cut:
  xor a
  ld (sms_since_cut),a
  ld (sms_lg),a
  ret

; Called after rendering the current trace. Requests become visible next step.
sms_finish_step:
  ld a,(sms_visible)
  or a
  jp z,sms_scene_advance
  ld a,(sms_entry_frame)
  or a
  jp nz,sms_scene_advance
  ld a,(sms_since_cut)
  cp 20
  jr nc,sms_finish_lg
  inc a
  ld (sms_since_cut),a
sms_finish_lg:
  ld a,(sms_lg)
  cp 10
  jr nc,sms_wire_gate
  inc a
  ld (sms_lg),a
sms_wire_gate:
  ld a,(sms_palette_ring)
  ld b,a
  ld a,(sms_wire_value)
  cp b
  jr z,sms_wire_window
  ld a,b
  ld (sms_wire_value),a
  xor a
  ld (sms_wire_count),a
  ld (sms_wire_used),a
sms_wire_window:
  ld a,(sms_scene_ring)
  cp 7
  jr z,sms_wire_count_step
  xor a
  ld (sms_wire_count),a
  ld (sms_request_valid),a
  jr sms_scene_advance
sms_wire_count_step:
  ld a,(sms_wire_count)
  cp 30
  jr nc,sms_wire_check
  inc a
  ld (sms_wire_count),a
sms_wire_check:
  cp 30
  jr nz,sms_request_check
  ld a,(sms_wire_used)
  or a
  jr nz,sms_request_check
  ld a,1
  ld (sms_wire_used),a
  ld a,(sms_wire_value)
  call sms_unarmed_palette
  ld (sms_request),a
  ld a,1
  ld (sms_request_valid),a
sms_request_check:
  ld a,(sms_request_valid)
  or a
  jr z,sms_scene_advance
  ld a,(sms_request)
  ld b,a
  ld a,(sms_peff)
  cp b
  jr nz,sms_request_budget
  xor a
  ld (sms_request_valid),a
  jr sms_scene_advance
sms_request_budget:
  ld a,(sms_since_cut)
  cp 20
  jr c,sms_scene_advance
  ld a,(sms_lg)
  cp 10
  jr c,sms_scene_advance
  ld a,b
  ld (sms_peff),a
  xor a
  ld (sms_request_valid),a
  ld (sms_since_cut),a
  ld (sms_lg),a
  inc a
  ld (sms_mode_changed),a

; Scene changes are decided after the current trace and apply next step.
sms_scene_advance:
  ld a,(sms_since_scene)
  cp 20
  jr nc,sms_scene_count
  inc a
  ld (sms_since_scene),a
sms_scene_count:
  ld a,(sms_scene_ring)
  ld b,a
  ld a,(sms_deb_value)
  cp b
  jr z,sms_scene_same
  ld a,b
  ld (sms_deb_value),a
  xor a
  ld (sms_deb_count),a
sms_scene_same:
  ld a,(sms_deb_count)
  cp 30
  jr nc,sms_scene_check
  inc a
  ld (sms_deb_count),a
sms_scene_check:
  cp 30
  ret c
  ld a,(sms_since_scene)
  cp 20
  ret c
  ld a,(sms_deb_value)
  ld b,0
  cp 7
  jr nz,sms_scene_compare
  inc b
sms_scene_compare:
  ld a,(sms_shown)
  cp b
  ret z
  ld a,b
  ld (sms_shown),a
  ld (sms_entry_pending),a
  xor a
  ld (sms_since_scene),a
  ld (sms_power_entry),a
  ret

; Clear a stale VBlank flag, then wait for the next one. No VDP writes.
; The caller must keep display disabled until a complete field is staged.
sms_wait_vblank:
  in a,($bf)
sms_wait_vblank_next:
  in a,($bf)
  and $80
  jr z,sms_wait_vblank_next
  ret

; Bring-up entry: A is the desired VDP register-1 value. The display is kept
; off indefinitely. This loop computes traces but does not render a field.
sms_hidden_poll_loop:
  di
  and $bf
  out ($bf),a
  ld a,$81
  out ($bf),a
  xor a
  call sms_control_init
  call sms_input_init
sms_hidden_poll_next:
  call sms_wait_vblank
  call sms_sample_pad
  call sms_make_input
  call sms_begin_step
  ld a,(sms_visible)
  or a
  jr z,sms_hidden_finish
  ld a,(sms_entry_frame)
  or a
  jr z,sms_hidden_later
  call sms_detector_entry
  call sms_field_scene_entry
  jr sms_hidden_trace
sms_hidden_later:
  call sms_detector_step
  call sms_field_step
sms_hidden_trace:
  call sms_field_make_trace
sms_hidden_finish:
  call sms_finish_step
  ld a,(sms_mode_changed)
  or a
  jr z,sms_hidden_poll_next
  call sms_field_mode_entry
  call sms_ack_mode_changed
  jr sms_hidden_poll_next

; Generated by field_assets.py; index = previous directions * 16 + current.
sms_pad_edge_table:
  .incbin "field-pad-edges.bin"
