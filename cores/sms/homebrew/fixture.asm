; Test-only callable image, not a bootable cartridge.
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
.INCLUDE "data_control.asm"
.INCLUDE "data_input.asm"
.INCLUDE "spectral_levels.asm"
.INCLUDE "safe_modes.asm"
.INCLUDE "data_detector.asm"
.INCLUDE "detector_fixture.asm"
.INCLUDE "field_data.asm"
