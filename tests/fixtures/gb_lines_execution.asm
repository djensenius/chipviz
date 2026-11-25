; Test-only Lines harness. Never included by the production ROM.
; Mailbox at $C000: ready, seq, ack, command, energy, look, four display
; shadows, four copied state bytes, last sequence. A new seq runs exactly one
; command at VBlank: 1 = LinesStep; 2 = corrupt VRAM then LinesLoad;
; 3 = present the prepared shadows to the LCD registers.
; $C100 holds a copy of the 512 tile and 2048 map bytes after each load.

DEF rLCDC EQU $FF40
DEF rLY   EQU $FF44
DEF rBGP  EQU $FF47
DEF rSCX  EQU $FF43
DEF rSCY  EQU $FF42

SECTION "Fixture mailbox", WRAM0[$C000]
wReady:    ds 1
wSeq:      ds 1
wAck:      ds 1
wCommand:  ds 1
wEnergy:   ds 1
wRequest:  ds 1
wShBGP:    ds 1
wShLCDC:   ds 1
wShSCX:    ds 1
wShSCY:    ds 1
wSnapshot: ds 4
wLastSeq:  ds 1

SECTION "Fixture VRAM copy", WRAM0[$C100]
wMirror: ds 512 + 2048

SECTION "Fixture entry", ROM0[$0100]
  nop
  jp FixtureMain
  ds $0150 - @, 0

SECTION "Fixture code", ROM0[$0150]
FixtureMain:
  di
  ld sp, $FFFE
  call FixtureVBlank
  xor a
  ldh [rLCDC], a
  ld hl, $C000
  ld bc, $2000
.clear
  ld [hli], a
  dec bc
  ld a, b
  or c
  jr z, .loaded
  xor a
  jr .clear
.loaded
  ; Sentinels prove LinesLoad does not change the output shadows.
  ld a, $12
  ld [wShBGP], a
  ld a, $34
  ld [wShLCDC], a
  ld a, $56
  ld [wShSCX], a
  ld a, $78
  ld [wShSCY], a
  call LinesLoad
  call FixtureCopyVram
  call FixtureSnapshot
  call FixtureDisplayOn
  ld a, $A5
  ld [wReady], a
.loop
  call FixtureVBlank
  ld a, [wSeq]
  ld b, a
  ld a, [wLastSeq]
  cp b
  jr z, .loop
  ld a, b
  ld [wLastSeq], a
  ld a, [wCommand]
  cp 3
  jr z, .present
  cp 2
  jr z, .reload
  cp 1
  jr nz, .ack
  ld a, [wEnergy]
  ld b, a
  ld a, [wRequest]
  call LinesStep
  jr .snapshot
.reload
  ; VBlank-only LCD disable; all VRAM accesses below happen with LCD off.
  xor a
  ldh [rLCDC], a
  ld hl, $8000
  ld bc, 512
  call FixtureCorrupt
  ld hl, $9800
  ld bc, 2048
  call FixtureCorrupt
  call LinesLoad
  call FixtureCopyVram
  call FixtureDisplayOn
.snapshot
  call FixtureSnapshot
.ack
  ld a, [wLastSeq]
  ld [wAck], a
  jr .loop
.present
  ; This test-only command runs immediately after FixtureVBlank. It performs
  ; four register writes and no VRAM writes; it does not time the real loop.
  ld a, [wShBGP]
  ldh [rBGP], a
  ld a, [wShSCY]
  ldh [rSCY], a
  ld a, [wShSCX]
  ldh [rSCX], a
  ld a, [wShLCDC]
  ldh [rLCDC], a
  jr .ack

FixtureVBlank:
.visible
  ldh a, [rLY]
  cp 144
  jr nc, .visible
.vblank
  ldh a, [rLY]
  cp 144
  jr c, .vblank
  ret

FixtureDisplayOn:
  xor a
  ldh [rSCX], a
  ldh [rSCY], a
  ld a, $E4
  ldh [rBGP], a
  ld a, $91
  ldh [rLCDC], a
  ret

FixtureCorrupt:
  ld a, $FF
.loop
  ld [hli], a
  dec bc
  ld a, b
  or c
  ret z
  ld a, $FF
  jr .loop

FixtureCopyVram:
  ld hl, wMirror
  ld de, $8000
  ld bc, 512
  call LinesCopy
  ld de, $9800
  ld bc, 2048
  jp LinesCopy

FixtureSnapshot:
  ld a, [wLinesLook]
  ld [wSnapshot], a
  ld a, [wLinesHold]
  ld [wSnapshot + 1], a
  ld a, [wLinesClock]
  ld [wSnapshot + 2], a
  ld a, [wLinesPhase]
  ld [wSnapshot + 3], a
  ret

INCLUDE "lines.asm"
