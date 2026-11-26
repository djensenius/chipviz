; Z80 data input for the colour-field detector, not a standalone ROM.
; Reserve $C020-$C033; include data_control.asm before this file.
;
; Initialise once with sms_input_init. Each step, after sms_sample_pad (or
; sms_sample), call sms_make_input. Before either button is first held it
; produces the shared procedural generator's detector fields, ignoring its
; scene/palette. From the first held button onward it uses the digital pad:
; energy and all bands 64, spectrum zero, button 1 = beat, button 2 = bar,
; no notes or shoulders. Flags are levels, not newly-pressed edges.
;
; Output layout: energy, bass, mid, treble, spectrum[8], flags, note_count,
; note, velocity. A future detector owns previous flags/notes and must seed
; them on scene entry. This module does not alter the wire frame format.

.define sms_input_energy    $c020
.define sms_input_spectrum  $c024
.define sms_input_flags     $c02c
.define sms_input_note_n    $c02d
.define sms_input_note      $c02e
.define sms_input_velocity  $c02f
.define sms_proc_low        $c030
.define sms_proc_high       $c031
.define sms_proc_note_index $c032
.define sms_pad_active      $c033

sms_input_init:
  ld hl,sms_input_energy
  ld b,$14
  xor a
sms_input_clear:
  ld (hl),a
  inc hl
  djnz sms_input_clear
  ret

sms_make_input:
  ld a,(sms_pad_active)
  or a
  jr nz,sms_input_pad
  ld a,(sms_buttons)
  and $30
  jr z,sms_input_procedural
  ld a,1
  ld (sms_pad_active),a
sms_input_pad:
  ld hl,sms_input_energy
  ld b,4
  ld a,64
sms_pad_bands:
  ld (hl),a
  inc hl
  djnz sms_pad_bands
  ld b,8
  xor a
sms_pad_spectrum:
  ld (hl),a
  inc hl
  djnz sms_pad_spectrum
  ld a,(sms_buttons)
  and $30
  rrca
  rrca
  rrca
  rrca
  ld (sms_input_flags),a
  xor a
  ld (sms_input_note_n),a
  ld (sms_input_note),a
  ld (sms_input_velocity),a
  ret

sms_input_procedural:
  ; A 13-byte row is indexed by the low byte of frame_index.
  ld a,(sms_proc_low)
  ld l,a
  ld h,0
  add hl,hl
  add hl,hl
  ld d,h
  ld e,l
  add hl,hl
  add hl,de
  ld a,(sms_proc_low)
  ld e,a
  ld d,0
  add hl,de
  ld de,sms_procedural_table
  add hl,de
  ld de,sms_input_energy
  ld bc,13
  ldir
  xor a
  ld (sms_input_note_n),a
  ld (sms_input_note),a
  ld (sms_input_velocity),a
  ld a,(sms_input_flags)
  or a
  jr z,sms_proc_advance
  ld a,1
  ld (sms_input_note_n),a
  ld a,(sms_proc_note_index)
  add a,48
  ld (sms_input_note),a
  ld a,(sms_input_energy)
  ld (sms_input_velocity),a
sms_proc_advance:
  ld hl,sms_proc_low
  inc (hl)
  jr nz,sms_proc_note_check
  inc hl
  inc (hl)
  jr nz,sms_proc_note_check
  xor a
  ld (sms_proc_note_index),a
  ret
sms_proc_note_check:
  ld a,(sms_proc_low)
  and $0f
  ret nz
  ld a,(sms_proc_note_index)
  inc a
  cp 24
  jr c,sms_proc_save_note
  xor a
sms_proc_save_note:
  ld (sms_proc_note_index),a
  ret

sms_procedural_table:
  .incbin "field-procedural.bin"
