; SMS DATA-role colour-field mode rules (held, spectral, unarmed cuts).
; Include data_control.asm, data_input.asm and spectral_levels.asm first.
; Reserve $C048-$C050 and $C060-$C067 in addition to those modules' RAM.
; This is not a standalone ROM and contains no flash/gate implementation.
;
; The detector sets sms_field_dom, sms_field_silent, and the X/D/E
; event bytes before calling sms_field_step. On scene entry initialise the
; detector, then call sms_field_scene_entry and sms_field_make_trace. On a
; palette change call sms_field_mode_entry immediately after sms_finish_step,
; using that step's inputs, and acknowledge sms_mode_changed. For later
; visible steps call sms_field_step, sms_field_make_trace, then
; sms_finish_step. Target execution and display timing remain unverified.

.define sms_field_combo       $c048
.define sms_field_rot         $c049
.define sms_field_act         $c04a
.define sms_field_dom         $c04b
.define sms_field_event_x     $c04c
.define sms_field_event_d     $c04d
.define sms_field_event_e     $c04e
.define sms_field_since_layout $c04f
.define sms_field_trace       $c060

; A = effective mode. Even a corrupted raw 3/7 never dispatches flash.
sms_field_mode:
  ld a,(sms_peff)
  and 3
  cp 3
  ret nz
  ld a,2
  ret

; New visit: auto layout starts at V2. Detector must already have seeded dom.
sms_field_scene_entry:
  xor a
  ld (sms_field_layout),a
  ld (sms_field_combo),a
  ld (sms_field_rot),a
  ld (sms_field_since_layout),a
  ld (sms_field_since_layout+1),a
sms_field_mode_entry:
  xor a
  ld (sms_field_act),a
  call sms_field_mode
  or a
  jr nz,sms_mode_not_held
  ld a,(sms_field_dom)
  ld (sms_field_combo),a
  ret
sms_mode_not_held:
  cp 1
  jp z,sms_spectral_init
  ret

; A = region count.
sms_field_regions:
  ld a,(sms_field_layout)
  and 2
  jr z,sms_field_two
  ld a,3
  ret
sms_field_two:
  ld a,2
  ret

sms_field_next_layout:
  ld a,(sms_field_layout)
  inc a
  and 3
  ld (sms_field_layout),a
  call sms_field_regions
  ld b,a
  ld a,(sms_field_rot)
  cp b
  jr c,sms_layout_keep_rot
  xor a
  ld (sms_field_rot),a
sms_layout_keep_rot:
  xor a
  ld (sms_field_since_layout),a
  ld (sms_field_since_layout+1),a
  ld (sms_lg),a
  call sms_field_mode
  cp 1
  jp z,sms_spectral_init
  ret

sms_field_next_combo:
  ld a,(sms_field_combo)
  inc a
  and 3
  ld (sms_field_combo),a
  ret

sms_field_rotate:
  call sms_field_regions
  ld b,a
  ld a,(sms_field_rot)
  inc a
  cp b
  jr c,sms_rotate_save
  xor a
sms_rotate_save:
  ld (sms_field_rot),a
  ret

; A = 1 for X, or D after 600 steps without a layout change; else 0.
sms_field_section:
  ld a,(sms_field_event_x)
  or a
  jr nz,sms_section_yes
sms_field_dom_section:
  ld a,(sms_field_event_d)
  or a
  jr z,sms_section_no
  ld hl,(sms_field_since_layout)
  ld a,h
  cp 2
  jr c,sms_section_no
  jr nz,sms_section_yes
  ld a,l
  cp 88
  jr c,sms_section_no
sms_section_yes:
  ld a,1
  ret
sms_section_no:
  xor a
  ret

; One later field step. Counters shared with palette gating are incremented
; only by sms_finish_step, after the trace is copied.
sms_field_step:
  ld a,(sms_field_silent)
  or a
  jr z,sms_field_rule
  call sms_field_mode
  cp 1
  call z,sms_spectral_update
  jr sms_field_count_layout
sms_field_rule:
  call sms_field_mode
  or a
  jr nz,sms_field_rule_spectral
  call sms_field_held
  jr sms_field_count_layout
sms_field_rule_spectral:
  cp 1
  jr nz,sms_field_rule_cuts
  call sms_field_spectral
  jr sms_field_count_layout
sms_field_rule_cuts:
  call sms_field_cuts
sms_field_count_layout:
  ld hl,(sms_field_since_layout)
  ld a,h
  and l
  cp $ff
  ret z
  inc hl
  ld (sms_field_since_layout),hl
  ret

sms_field_held:
  ld a,(sms_field_event_d)
  or a
  jr z,sms_held_section
  ld a,(sms_field_dom)
  ld (sms_field_combo),a
  call sms_visible_change
sms_held_section:
  call sms_field_section
  or a
  ret z
  ld a,(sms_since_cut)
  cp 20
  ret c
  call sms_field_next_layout
  jp sms_cut

sms_field_spectral:
  ld a,(sms_lg)
  cp 10
  ret c
  ld a,(sms_field_event_x)
  or a
  jr z,sms_spectral_section
  ld a,(sms_since_cut)
  cp 20
  jr c,sms_spectral_section
  call sms_field_next_layout
  call sms_field_next_combo
  jp sms_cut
sms_spectral_section:
  call sms_field_dom_section
  or a
  jp nz,sms_field_next_layout
sms_spectral_levels_only:
  jp sms_spectral_update

sms_field_cuts:
  ld a,(sms_field_event_e)
  or a
  ret z
  ld a,(sms_since_cut)
  cp 20
  ret c
  call sms_cut
  ld a,(sms_field_event_x)
  or a
  jr z,sms_cuts_action
  call sms_field_next_layout
  jp sms_field_next_combo
sms_cuts_action:
  ld a,(sms_field_act)
  ld e,a
  ld d,0
  inc a
  and 3
  ld (sms_field_act),a
  ld hl,sms_cut_action_table
  add hl,de
  ld a,(hl)
  or a
  jp z,sms_field_rotate
  cp 1
  jp z,sms_field_next_combo
  jp sms_field_next_layout

; Snapshot (mode, family, layout, combo, rot, level0..2) before requests.
sms_field_make_trace:
  ld hl,sms_field_trace
  call sms_field_mode
  ld c,a
  ld (hl),a
  inc hl
  ld a,(sms_peff)
  srl a
  srl a
  ld (hl),a
  inc hl
  ld a,(sms_field_layout)
  ld (hl),a
  inc hl
  ld a,(sms_field_combo)
  ld (hl),a
  inc hl
  ld a,(sms_field_rot)
  ld (hl),a
  inc hl
  ld a,c
  cp 1
  jr z,sms_trace_spectral
  ld (hl),3
  inc hl
  ld (hl),3
  inc hl
  ld a,(sms_field_layout)
  and 2
  ld a,0
  jr z,sms_trace_last
  ld a,3
sms_trace_last:
  ld (hl),a
  ret
sms_trace_spectral:
  ld de,sms_field_levels
  ld b,3
sms_trace_copy:
  ld a,(de)
  ld (hl),a
  inc de
  inc hl
  djnz sms_trace_copy
  ret

sms_cut_action_table:
  .incbin "field-cut-actions.bin"
