; SMS data-role spectral level state (WLA-Z80). Not a standalone ROM.
; Include data_control.asm and data_input.asm first.
;
; Reserve $C040-$C047. The caller supplies layout 0..3, silence 0/1 and
; input bands, and maintains sms_lg via data_control.asm. Levels are 0..3;
; level 2 is always 0 for two-region layouts.
;
; sms_spectral_init is for scene/mode entry or a layout change. The caller
; applies the appropriate change budget reset. sms_spectral_update handles
; only spectral blackout and level hysteresis; the caller must first handle
; higher-priority X/D layout changes and must not call update in a step where
; a layout change reset the field-wide budget. Neither routine presents pixels.

.define sms_field_layout $c040
.define sms_field_silent $c041
.define sms_field_levels $c042
.define sms_field_values $c045

; Derive bass / max(mid,treble) for two regions, or bass/mid/treble for three.
sms_spectral_values:
  ld a,(sms_input_energy+1)
  ld (sms_field_values),a
  ld a,(sms_input_energy+2)
  ld b,a
  ld a,(sms_field_layout)
  and 2
  jr nz,sms_spectral_three_values
  ld a,(sms_input_energy+3)
  cp b
  jr nc,sms_spectral_save_second
  ld a,b
sms_spectral_save_second:
  ld (sms_field_values+1),a
  xor a
  ld (sms_field_values+2),a
  ret
sms_spectral_three_values:
  ld a,b
  ld (sms_field_values+1),a
  ld a,(sms_input_energy+3)
  ld (sms_field_values+2),a
  ret

; Initialise levels from raw bands. Silent spectral entry starts black.
sms_spectral_init:
  xor a
  ld (sms_field_levels),a
  ld (sms_field_levels+1),a
  ld (sms_field_levels+2),a
  ld a,(sms_field_silent)
  or a
  ret nz
  call sms_spectral_values
  ld ix,sms_field_levels
  ld iy,sms_field_values
  ld b,2
  ld a,(sms_field_layout)
  and 2
  jr z,sms_spectral_init_loop
  inc b
sms_spectral_init_loop:
  ld e,(iy+0)
  ld d,0
  ld hl,sms_spectral_initial_table
  add hl,de
  ld a,(hl)
  ld (ix+0),a
  inc ix
  inc iy
  djnz sms_spectral_init_loop
  ret

; At most one field-wide update per 10 steps. Silence blackout also uses lg.
sms_spectral_update:
  ld a,(sms_lg)
  cp 10
  ret c
  ld a,(sms_field_silent)
  or a
  jr z,sms_spectral_hysteresis
  ld hl,sms_field_levels
  ld a,(hl)
  inc hl
  or (hl)
  inc hl
  or (hl)
  ret z
  xor a
  ld (sms_field_levels),a
  ld (sms_field_levels+1),a
  ld (sms_field_levels+2),a
  jp sms_visible_change
sms_spectral_hysteresis:
  call sms_spectral_values
  ld ix,sms_field_levels
  ld iy,sms_field_values
  ld b,2
  ld a,(sms_field_layout)
  and 2
  jr z,sms_spectral_start_loop
  inc b
sms_spectral_start_loop:
  ld c,0
sms_spectral_loop:
  ld d,(ix+0)
  ld e,(iy+0)
  ld hl,sms_spectral_next_table
  add hl,de
  ld a,(ix+0)
  cp (hl)
  jr z,sms_spectral_next
  ld a,(hl)
  ld (ix+0),a
  ld c,1
sms_spectral_next:
  inc ix
  inc iy
  djnz sms_spectral_loop
  ld a,c
  or a
  ret z
  jp sms_visible_change

sms_spectral_initial_table:
  .incbin "field-spectral-initial.bin"
sms_spectral_next_table:
  .incbin "field-spectral-next.bin"
