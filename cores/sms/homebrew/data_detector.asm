; SMS DATA-role detector for the input format in data_input.asm.
; Include data_control.asm, data_input.asm, spectral_levels.asm and
; safe_modes.asm first. Reserve $C070-$C095. This is not a standalone ROM.
;
; The current input path carries at most one (note, velocity) pair, not the
; full control-frame's four notes. On each colour-field entry, call
; sms_detector_entry before sms_field_scene_entry. On each later field step,
; call sms_detector_step before sms_field_step. It supplies field_silent,
; field_dom and X/D/E; entry seeds previous input and raises no events.
; Palette requests and scene debounce remain in data_control.asm.

.define sms_det_acc       $c070
.define sms_det_delta     $c078
.define sms_det_prev_spec $c07c
.define sms_det_prev_flag $c084
.define sms_det_prev_n    $c085
.define sms_det_prev_note $c086
.define sms_det_prev_vel  $c087
.define sms_det_armed     $c088
.define sms_det_quiet     $c089
.define sms_det_cand      $c08a
.define sms_det_cand_n    $c08b
.define sms_det_flux      $c08c
.define sms_det_event_t   $c08e
.define sms_det_event_n   $c08f
.define sms_det_event_b   $c090
.define sms_det_avg_band  $c091
.define sms_det_input_tmp $c094
.define sms_det_pick_tmp  $c095

sms_detector_entry:
  ; Four EMA accumulators start at input << 3.
  ld ix,sms_input_energy
  ld iy,sms_det_acc
  ld b,4
sms_det_seed_acc:
  ld l,(ix+0)
  ld h,0
  add hl,hl
  add hl,hl
  add hl,hl
  ld (iy+0),l
  ld (iy+1),h
  inc ix
  inc iy
  inc iy
  djnz sms_det_seed_acc
  ld hl,sms_input_spectrum
  ld de,sms_det_prev_spec
  ld bc,8
  ldir
  call sms_det_save_previous
  xor a
  ld (sms_field_silent),a
  ld (sms_det_quiet),a
  ld (sms_det_cand_n),a
  ld (sms_field_event_x),a
  ld (sms_field_event_d),a
  ld (sms_field_event_e),a
  ld (sms_det_event_t),a
  ld (sms_det_event_n),a
  ld (sms_det_event_b),a
  ld a,1
  ld (sms_det_armed),a
  ld a,$ff
  ld (sms_det_cand),a
  ld hl,sms_input_energy+1
  call sms_det_argmax
  ld (sms_field_dom),a
  ret

; C = accumulator index 0..3; A = accumulator >> 3. C is preserved.
sms_det_average:
  ld a,c
  add a,a
  ld e,a
  ld d,0
  ld hl,sms_det_acc
  add hl,de
  ld a,(hl)
  inc hl
  ld h,(hl)
  ld l,a
  srl h
  rr l
  srl h
  rr l
  srl h
  rr l
  ld a,l
  ret

; HL points to three unsigned bytes; return the first index at the maximum.
sms_det_argmax:
  ld c,0
  ld b,(hl)
  inc hl
  ld a,(hl)
  cp b
  jr c,sms_det_argmax_third
  jr z,sms_det_argmax_third
  ld b,a
  inc c
sms_det_argmax_third:
  inc hl
  ld a,(hl)
  cp b
  jr c,sms_det_argmax_done
  jr z,sms_det_argmax_done
  ld c,2
sms_det_argmax_done:
  ld a,c
  ret

sms_det_save_previous:
  ld a,(sms_input_flags)
  ld (sms_det_prev_flag),a
  ld a,(sms_input_note_n)
  ld (sms_det_prev_n),a
  ld a,(sms_input_note)
  ld (sms_det_prev_note),a
  ld a,(sms_input_velocity)
  ld (sms_det_prev_vel),a
  ret

sms_detector_step:
  ; Old-average deltas are clipped at zero: all transient comparisons are
  ; against positive thresholds, so negative deltas are equivalent to zero.
  ld c,0
sms_det_ema_loop:
  ld e,c
  ld d,0
  ld hl,sms_input_energy
  add hl,de
  ld a,(hl)
  ld (sms_det_input_tmp),a
  call sms_det_average
  ld b,a
  ld a,(sms_det_input_tmp)
  sub b
  jr nc,sms_det_delta_positive
  xor a
sms_det_delta_positive:
  ld e,c
  ld d,0
  ld hl,sms_det_delta
  add hl,de
  ld (hl),a
  ld a,c
  add a,a
  ld e,a
  ld d,0
  ld hl,sms_det_acc
  add hl,de
  ld a,(hl)
  sub b
  ld (hl),a
  inc hl
  ld a,(hl)
  sbc a,0
  ld (hl),a
  dec hl
  ld a,(sms_det_input_tmp)
  add a,(hl)
  ld (hl),a
  jr nc,sms_det_ema_next
  inc hl
  inc (hl)
sms_det_ema_next:
  inc c
  ld a,c
  cp 4
  jr nz,sms_det_ema_loop

  ; Positive spectrum flux and previous input.
  xor a
  ld (sms_det_flux),a
  ld (sms_det_flux+1),a
  ld ix,sms_input_spectrum
  ld iy,sms_det_prev_spec
  ld b,8
sms_det_flux_loop:
  ld c,(iy+0)
  ld a,(ix+0)
  ld (iy+0),a
  sub c
  jr c,sms_det_flux_next
  ld c,a
  ld a,(sms_det_flux)
  add a,c
  ld (sms_det_flux),a
  jr nc,sms_det_flux_next
  ld a,(sms_det_flux+1)
  inc a
  ld (sms_det_flux+1),a
sms_det_flux_next:
  inc ix
  inc iy
  djnz sms_det_flux_loop

  ; Silence entry uses the updated average; exit uses raw energy.
  ld a,(sms_field_silent)
  or a
  jr z,sms_det_update_quiet
  ld a,(sms_input_energy)
  cp 24
  jr c,sms_det_transient
  xor a
  ld (sms_field_silent),a
  ld (sms_det_quiet),a
  jr sms_det_transient
sms_det_update_quiet:
  ld c,0
  call sms_det_average
  cp 13
  jr c,sms_det_quiet_count
  xor a
  ld (sms_det_quiet),a
  jr sms_det_transient
sms_det_quiet_count:
  ld a,(sms_det_quiet)
  cp 30
  jr nc,sms_det_silence_set
  inc a
  ld (sms_det_quiet),a
  cp 30
  jr c,sms_det_transient
sms_det_silence_set:
  ld a,1
  ld (sms_field_silent),a

sms_det_transient:
  xor a
  ld (sms_det_event_t),a
  ld a,(sms_input_energy)
  cp 24
  jr c,sms_det_calm
  ld a,(sms_det_delta)
  cp 40
  jr nc,sms_det_hit
  ld a,(sms_det_delta+1)
  cp 56
  jr nc,sms_det_hit
  ld a,(sms_det_delta+2)
  cp 56
  jr nc,sms_det_hit
  ld a,(sms_det_delta+3)
  cp 56
  jr nc,sms_det_hit
  ld a,(sms_det_flux+1)
  or a
  jr nz,sms_det_hit
  ld a,(sms_det_flux)
  cp 200
  jr c,sms_det_calm
sms_det_hit:
  ld a,(sms_det_armed)
  or a
  jr z,sms_det_events
  xor a
  ld (sms_det_armed),a
  inc a
  ld (sms_det_event_t),a
  jr sms_det_events
sms_det_calm:
  ld a,(sms_det_delta)
  cp 16
  jr nc,sms_det_events
  ld a,(sms_det_delta+1)
  cp 24
  jr nc,sms_det_events
  ld a,(sms_det_delta+2)
  cp 24
  jr nc,sms_det_events
  ld a,(sms_det_delta+3)
  cp 24
  jr nc,sms_det_events
  ld a,(sms_det_flux+1)
  or a
  jr nz,sms_det_events
  ld a,(sms_det_flux)
  cp 64
  jr nc,sms_det_events
  ld a,1
  ld (sms_det_armed),a

sms_det_events:
  xor a
  ld (sms_det_event_n),a
  ld a,(sms_input_note_n)
  or a
  jr z,sms_det_flag_edges
  ld a,(sms_input_velocity)
  cp 64
  jr c,sms_det_flag_edges
  ld b,a
  ld a,(sms_det_prev_n)
  or a
  jr z,sms_det_note_onset
  ld a,(sms_det_prev_vel)
  cp b
  jr nz,sms_det_note_onset
  ld a,(sms_det_prev_note)
  ld b,a
  ld a,(sms_input_note)
  cp b
  jr z,sms_det_flag_edges
sms_det_note_onset:
  ld a,1
  ld (sms_det_event_n),a
sms_det_flag_edges:
  ld a,(sms_det_prev_flag)
  cpl
  ld b,a
  ld a,(sms_input_flags)
  and b
  ld b,a
  and 3
  jr z,sms_det_save_beat
  ld a,1
sms_det_save_beat:
  ld (sms_det_event_b),a
  ld a,b
  and 4
  jr z,sms_det_save_drop
  ld a,1
sms_det_save_drop:
  ld (sms_field_event_x),a
  call sms_det_save_previous
  xor a
  ld (sms_field_event_e),a
  ld a,(sms_field_silent)
  or a
  jr nz,sms_det_dominance
  ld a,(sms_det_event_t)
  ld b,a
  ld a,(sms_det_event_n)
  or b
  ld b,a
  ld a,(sms_det_event_b)
  or b
  ld b,a
  ld a,(sms_field_event_x)
  or b
  ld (sms_field_event_e),a

sms_det_dominance:
  ld c,1
  call sms_det_average
  ld (sms_det_avg_band),a
  ld c,2
  call sms_det_average
  ld (sms_det_avg_band+1),a
  ld c,3
  call sms_det_average
  ld (sms_det_avg_band+2),a
  ld hl,sms_det_avg_band
  call sms_det_argmax
  ld (sms_det_pick_tmp),a
  xor a
  ld (sms_field_event_d),a
  ld a,(sms_field_silent)
  or a
  jr nz,sms_det_cand_reset
  ld a,(sms_field_dom)
  ld b,a
  ld a,(sms_det_pick_tmp)
  cp b
  jr z,sms_det_cand_reset
  ld e,b
  ld d,0
  ld hl,sms_det_avg_band
  add hl,de
  ld a,(hl)
  add a,32
  jr c,sms_det_cand_reset
  ld b,a
  ld a,(sms_det_pick_tmp)
  ld e,a
  ld d,0
  ld hl,sms_det_avg_band
  add hl,de
  ld a,(hl)
  cp b
  jr c,sms_det_cand_reset
  ld a,(sms_det_pick_tmp)
  ld b,a
  ld a,(sms_det_cand)
  cp b
  jr z,sms_det_cand_increment
  ld a,b
  ld (sms_det_cand),a
  ld a,1
  ld (sms_det_cand_n),a
  ret
sms_det_cand_increment:
  ld a,(sms_det_cand_n)
  cp 30
  jr nc,sms_det_dom_set
  inc a
  ld (sms_det_cand_n),a
  cp 30
  ret c
sms_det_dom_set:
  ld a,(sms_det_pick_tmp)
  ld (sms_field_dom),a
  xor a
  ld (sms_det_cand_n),a
  inc a
  ld (sms_field_event_d),a
  ret
sms_det_cand_reset:
  ld a,$ff
  ld (sms_det_cand),a
  xor a
  ld (sms_det_cand_n),a
  ret
