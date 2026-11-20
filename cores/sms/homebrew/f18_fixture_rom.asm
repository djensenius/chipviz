; Test-only bootable driver for the existing callable detector fixture.
; Fixture inputs are embedded, one row per VBlank. Reading $DC marks a
; completed mailbox for the host observer; the returned pad value is ignored.
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
  jp f18_reset
.ORG $38
  reti
.ORG $66
  retn
.ORG $100
f18_reset:
  di
  im 1
  ld sp,$dff0
  xor a
  ld ($fffc),a
  ld ($fffd),a
  inc a
  ld ($fffe),a
  inc a
  ld ($ffff),a
  ld a,$a8
  out ($3e),a
  ld a,$ff
  out ($3f),a
  xor a
  ld ($c110),a
  ld ($c111),a
  ld a,7
  call sms_fixture_init
  ld hl,f18_rows
  ld ($c112),hl
f18_step:
  ld hl,($c112)
  ld a,(hl)
  ld (sms_fx_scene),a
  inc hl
  ld a,(hl)
  ld (sms_fx_palette),a
  inc hl
  ld de,sms_fx_input
  ld bc,16
  ldir
  ld ($c112),hl
  call sms_fixture_step
f18_emit:
  in a,($dc)
  ld hl,($c110)
  inc hl
  ld ($c110),hl
  ld de,600
  or a
  sbc hl,de
  jr z,f18_done
  call sms_wait_vblank
  jr f18_step
f18_done:
  halt
  jr f18_done

.INCLUDE "data_control.asm"
.INCLUDE "data_input.asm"
.INCLUDE "spectral_levels.asm"
.INCLUDE "safe_modes.asm"
.INCLUDE "data_detector.asm"
.INCLUDE "detector_fixture.asm"
.INCLUDE "field_data.asm"
f18_rows:
  .incbin "f18.input.bin"
.ORG $7ff0
  .dsb 16,$a5
