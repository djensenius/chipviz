; chipviz Game Boy (DMG): native tile-field demo (scene 0) and the colour
; field (scene 7, shared/specs/colour-field-v0.md, DMG gating variant).
;
; One step runs per frame. At each VBlank the main loop first writes the
; display registers prepared by the previous step (BGP, LCDC, SCX, SCY), so a
; step's whole output appears at once. Then it reads the pad and runs the step.
;
; Build flags (see Makefile):
;   -D CHIPVIZ_INPUT_ROLE_OPERATOR  operator input role (default: data role)
;   -D CHIPVIZ_FIXTURE              emulator test build: steps read a WRAM
;                                   mailbox (the full control-frame path)

IF DEF(CHIPVIZ_INPUT_ROLE_OPERATOR) && DEF(CHIPVIZ_FIXTURE)
  FAIL "CHIPVIZ_FIXTURE is a data-role test build"
ENDC

DEF rP1   EQU $FF00
DEF rLCDC EQU $FF40
DEF rSCY  EQU $FF42
DEF rSCX  EQU $FF43
DEF rLY   EQU $FF44
DEF rBGP  EQU $FF47

; Pad bits as ReadPad packs them.
DEF PAD_A      EQU $01
DEF PAD_B      EQU $02
DEF PAD_SELECT EQU $04
DEF PAD_START  EQU $08
DEF PAD_RIGHT  EQU $10
DEF PAD_LEFT   EQU $20
DEF PAD_UP     EQU $40
DEF PAD_DOWN   EQU $80

DEF LCDC_NATIVE EQU %10010001     ; BG map $9800, tiles $8000

SECTION "Entry", ROM0[$0100]
  nop
  jp Main
  ds $0150 - @, 0

SECTION "Main", ROM0[$0150]
Main:
  di
  ld sp, $fffe
  call WaitVBlank
  xor a
  ldh [rLCDC], a
  ; Clear WRAM. The stack lives in HRAM.
  ld hl, $C000
  ld bc, $2000
.clear
  xor a
  ld [hli], a
  dec bc
  ld a, b
  or c
  jr nz, .clear
  call LoadTiles
  call LoadMap
  call LoadFieldVram
  call InitState
  ld a, %11100100
  ldh [rBGP], a
  ld [wShBGP], a
  ld a, LCDC_NATIVE
  ld [wShLCDC], a
  ldh [rLCDC], a
IF DEF(CHIPVIZ_FIXTURE)
  ld a, FIXTURE_READY
  ld [wReady], a
ENDC
Loop:
  call WaitVBlank
  call ApplyDisplay
IF DEF(CHIPVIZ_FIXTURE)
  ; Full control-frame path: one step per new mailbox frame counter.
  ld a, [wLastSeq]
  ld b, a
  ld a, [wFxSeq]
  cp b
  jr z, Loop
  ld [wLastSeq], a
ENDC
  call ReadPad
  call Step
IF DEF(CHIPVIZ_FIXTURE)
  ld a, [wLastSeq]
  ld [wAckSeq], a
ENDC
  jr Loop

WaitVBlank:
.waitVisible
  ldh a, [rLY]
  cp 144
  jr nc, .waitVisible
.waitVBlank
  ldh a, [rLY]
  cp 144
  jr c, .waitVBlank
  ret

; VBlank: write the registers the last step prepared and record what is shown.
ApplyDisplay:
  ld a, [wShBGP]
  ldh [rBGP], a
  ld a, [wShSCY]
  ldh [rSCY], a
  ld a, [wShSCX]
  ldh [rSCX], a
  ld a, [wShLCDC]
  ldh [rLCDC], a
  ld hl, wTraceShow
  ld de, wShownShow
  ld c, 9
.copy
  ld a, [hli]
  ld [de], a
  inc de
  dec c
  jr nz, .copy
  ret

; Native scene: the original scrolling tile field and its palette cycle.
NativeStep:
  ld hl, wFrame
  inc [hl]
  ld a, [hl]
  ld [wShSCY], a
  rrca
  ld [wShSCX], a
  and %00011000
  or %11100100
  ld [wShBGP], a
  ld a, LCDC_NATIVE
  ld [wShLCDC], a
  xor a
  ld [wTraceShow], a
  ret

; wPad bits: 7 Down, 6 Up, 5 Left, 4 Right, 3 Start, 2 Select, 1 B, 0 A.
ReadPad:
  ld a, $20
  ldh [rP1], a
  ldh a, [rP1]
  ldh a, [rP1]
  cpl
  and $0F
  swap a
  ld b, a
  ld a, $10
  ldh [rP1], a
  ldh a, [rP1]
  ldh a, [rP1]
  ldh a, [rP1]
  ldh a, [rP1]
  ldh a, [rP1]
  ldh a, [rP1]
  cpl
  and $0F
  or b
  ld b, a
  ld a, $30
  ldh [rP1], a
  ld a, [wPad]
  cpl
  and b
  ld [wPadNew], a
  ld a, b
  ld [wPad], a
  ret

LoadTiles:
  ld hl, $8000
  ld de, Tiles
  ld bc, TilesEnd - Tiles
.copy
  ld a, [de]
  ld [hli], a
  inc de
  dec bc
  ld a, b
  or c
  jr nz, .copy
  ret

LoadMap:
  ld hl, $9800
  ld bc, 32 * 32
  xor a
.copy
  ld [hli], a
  inc a
  and 3
  dec bc
  ld d, a
  ld a, b
  or c
  ld a, d
  jr nz, .copy
  ret

SECTION "Native state", WRAM0
wFrame:
  ds 1

SECTION "Tiles", ROM0
Tiles:
  db $00,$00,$18,$18,$3c,$3c,$7e,$7e,$ff,$ff,$7e,$7e,$3c,$3c,$18,$18
  db $81,$81,$42,$42,$24,$24,$18,$18,$18,$18,$24,$24,$42,$42,$81,$81
  db $ff,$00,$81,$7e,$bd,$42,$a5,$5a,$a5,$5a,$bd,$42,$81,$7e,$ff,$00
  db $11,$ee,$22,$dd,$44,$bb,$88,$77,$11,$ee,$22,$dd,$44,$bb,$88,$77
TilesEnd:

INCLUDE "colour_field.asm"
