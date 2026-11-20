; Experimental SMS ROM. Independently written startup; no SDK runtime.
.MEMORYMAP
  DEFAULTSLOT 0
  SLOTSIZE $8000
  SLOT 0 $0000
.ENDME
.ROMBANKSIZE $8000
.ROMBANKS 1
.EMPTYFILL $00
.BANK 0 SLOT 0
.ORG 0
  di
  jp sms_reset
.ORG $38
  reti
.ORG $66
  retn
.ORG $100

; Presenter scratch, disjoint from the detector and fixture mailboxes.
.define sms_present_register $c100
.define sms_present_colours  $c101
.define sms_present_regions  $c104
.define sms_present_i        $c105
.define sms_present_combo    $c106

sms_reset:
  di
  im 1
  ld sp,$dff0
  ; Initialise standard Sega mapper pages before accessing ROM above $3fff.
  xor a
  ld ($fffc),a
  ld ($fffd),a
  inc a
  ld ($fffe),a
  inc a
  ld ($ffff),a
  ; Disable BIOS, card and expansion; leave cartridge and I/O enabled.
  ld a,$a8
  out ($3e),a
  ld a,$ff
  out ($3f),a
  ; Mute all PSG channels.
  ld a,$9f
  out ($7f),a
  ld a,$bf
  out ($7f),a
  ld a,$df
  out ($7f),a
  ld a,$ff
  out ($7f),a
  ; Mode 4, 192 lines, display and VDP interrupts off while building VRAM.
  ld hl,sms_vdp_registers
  ld c,$80
  ld b,11
sms_register_loop:
  ld a,(hl)
  out ($bf),a
  ld a,c
  out ($bf),a
  inc hl
  inc c
  djnz sms_register_loop

  ; Four solid tile patterns at $0000.
  ld hl,sms_field_patterns
  ld de,$4000
  ld bc,128
  call sms_copy_vram
  ; Native table at $1800, all tile 1 (white). No overlapping sprite table.
  ld de,$5800
  call sms_vram_address
  ld bc,$0380
sms_native_clear:
  ld a,1
  out ($be),a
  xor a
  ; Leave ample time between VDP data-port writes even on real hardware.
  nop
  nop
  nop
  out ($be),a
  dec bc
  ld a,b
  or c
  jr z,sms_native_done
  jr sms_native_clear
sms_native_done:
  ; End-of-list marker at sprite table $1f00.
  ld de,$5f00
  call sms_vram_address
  ld a,$d0
  out ($be),a
  ; Four complete 2 KiB-aligned field maps, $2000 through $3fff.
  ld hl,sms_field_maps
  ld de,$6000
  ld bc,$2000
  call sms_copy_vram
  ; Initialise all CRAM; native tile 1 is white, backdrop is black.
  xor a
  out ($bf),a
  ld a,$c0
  out ($bf),a
  xor a
  out ($be),a
  ld a,$3f
  out ($be),a
  ld b,30
  xor a
sms_clear_cram:
  out ($be),a
  djnz sms_clear_cram
  xor a
  call sms_control_init
  call sms_input_init
  ; First visible frame is the fully prepared native table.
  call sms_wait_vblank
  ld a,$c0
  out ($bf),a
  ld a,$81
  out ($bf),a
sms_main_loop:
  call sms_sample_pad
  call sms_make_input
  call sms_begin_step
  ld a,(sms_visible)
  or a
  jr z,sms_prepare_native
  ld a,(sms_entry_frame)
  or a
  jr z,sms_main_later
  call sms_detector_entry
  call sms_field_scene_entry
  jr sms_main_trace
sms_main_later:
  call sms_detector_step
  call sms_field_step
sms_main_trace:
  call sms_field_make_trace
  call sms_prepare_field
  jr sms_main_finish
sms_prepare_native:
  ld a,$f7
  ld (sms_present_register),a
  ld a,$3f
  ld (sms_present_colours),a
  xor a
  ld (sms_present_colours+1),a
  ld (sms_present_colours+2),a
sms_main_finish:
  call sms_finish_step
  ld a,(sms_mode_changed)
  or a
  jr z,sms_main_publish
  call sms_field_mode_entry
  call sms_ack_mode_changed
sms_main_publish:
  ; All expensive work is done. Begin publication at a fresh VBlank edge:
  ; three CRAM entries followed by one name-table register write.
  call sms_wait_vblank
  ld a,1
  out ($bf),a
  ld a,$c0
  out ($bf),a
  ld hl,sms_present_colours
  ld b,3
sms_publish_colours:
  ld a,(hl)
  out ($be),a
  inc hl
  djnz sms_publish_colours
sms_publish_map:
  ld a,(sms_present_register)
  out ($bf),a
  ld a,$82
  out ($bf),a
  jp sms_main_loop

; DE = VRAM write command, HL = source, BC = count. Display is disabled.
sms_copy_vram:
  call sms_vram_address
sms_copy_vram_loop:
  ld a,(hl)
  out ($be),a
  inc hl
  dec bc
  ld a,b
  or c
  jr nz,sms_copy_vram_loop
  ret
sms_vram_address:
  ld a,e
  out ($bf),a
  ld a,d
  out ($bf),a
  ret

; Trace -> staged CRAM values. Colours rotate; region levels do not.
sms_prepare_field:
  ld a,(sms_field_trace+2)
  add a,a
  add a,$f9
  ld (sms_present_register),a
  ld a,(sms_field_trace+2)
  and 2
  ld b,2
  jr z,sms_present_count
  inc b
sms_present_count:
  ld a,b
  ld (sms_present_regions),a
  ld a,(sms_field_trace+1)
  add a,a
  add a,a
  add a,a
  add a,a
  add a,a
  ld c,a
  ld a,(sms_field_trace+2)
  and 2
  rlca
  rlca
  rlca
  add a,c
  ld c,a
  ld a,(sms_field_trace+3)
  add a,a
  add a,a
  add a,c
  ld (sms_present_combo),a
  xor a
  ld (sms_present_i),a
  ld (sms_present_colours+2),a
sms_present_loop:
  ld a,(sms_field_trace+4)
  ld b,a
  ld a,(sms_present_i)
  add a,b
  ld b,a
  ld a,(sms_present_regions)
  cp b
  jr z,sms_present_wrap
  jr nc,sms_present_index
sms_present_wrap:
  ld a,b
  ld b,a
  ld a,(sms_present_regions)
  ld c,a
  ld a,b
  sub c
  ld b,a
sms_present_index:
  ld a,(sms_present_combo)
  add a,b
  ld e,a
  ld d,0
  ld hl,sms_field_combos
  add hl,de
  ld a,(hl)
  add a,a
  add a,a
  ld b,a
  ld a,(sms_present_i)
  ld e,a
  ld d,0
  ld hl,sms_field_trace+5
  add hl,de
  ld a,(hl)
  add a,b
  ld e,a
  ld hl,sms_field_cram_levels
  add hl,de
  ld a,(hl)
  ld b,a
  ld a,(sms_present_i)
  ld e,a
  ld hl,sms_present_colours
  add hl,de
  ld (hl),b
  ld a,(sms_present_i)
  inc a
  ld (sms_present_i),a
  ld b,a
  ld a,(sms_present_regions)
  cp b
  jr nz,sms_present_loop
  ret

; R2=$f7 -> $1800; R5=$3e -> $1f00.
sms_vdp_registers:
  .db $04,$80,$f7,$ff,$ff,$3e,$00,$00,$00,$00,$ff

.INCLUDE "data_control.asm"
.INCLUDE "data_input.asm"
.INCLUDE "spectral_levels.asm"
.INCLUDE "safe_modes.asm"
.INCLUDE "data_detector.asm"
.INCLUDE "field_data.asm"

; Reserve the header so the assembler rejects ROM content that overruns it.
.ORG $7ff0
  .dsb 16,$a5
