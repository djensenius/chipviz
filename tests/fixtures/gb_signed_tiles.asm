; Test-only signed-tile ownership experiment, not production dispatch.
; The host redirects this temporary ROM's entry to SignedMain.
; Diagnostic tiles are installed at $9000 with LCD off at boot. No runtime
; tile update or VBlank copy budget is tested. The native map is not modified.
; The optional freeze stops ReadPad and Step (including scene-7 budgets) for
; still-frame comparisons only. It is not a production handoff or safety model.
; The selector is a test mailbox, not a catalog request or production queue.
; Scene 7 and warning output always take priority and clear that mailbox.
;
; With LCD off, two observation buffers record $8000-$81FF, $8800-$897F,
; $9100-$91FF and both complete maps, before staging and after the test.
; These include the native and field tiles, unknown $8040-$80FF, operator
; warning font, and both maps. Linker placement must verify all reservations.

INCLUDE "main.asm"

SECTION "Signed mailbox", WRAM0[$C040]
wSignedReady:   ds 1
wSignedRequest: ds 1
wSignedActive:  ds 1       ; signed output prepared, not necessarily presented
wSignedRefused: ds 1
wSignedFreeze:  ds 1
wSignedInspect: ds 1
wSignedDone:    ds 1

SECTION "Signed observation after", WRAM0[$C300]
wSignedAfter: ds 3200

SECTION "Signed observation before", WRAMX[$D000], BANK[1]
wSignedBefore: ds 3200

SECTION "Signed fixture", ROM0
SignedMain:
  di
  ld sp, $FFFE
  call WaitVBlank
  xor a
  ldh [rLCDC], a
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
  ; These unknown bytes are not supplied by either production loader.
  ld hl, $8040
  ld b, 192
  ld a, $35
.seed
  ld [hli], a
  add 17
  dec b
  jr nz, .seed
  ld hl, wSignedBefore
  call SignedObserve
  ld hl, $9000
  ld de, SignedTiles
  ld bc, 64
  call SignedCopy
  ld a, $E4
  ldh [rBGP], a
  ld [wShBGP], a
  ld a, LCDC_NATIVE
  ld [wShLCDC], a
  ldh [rLCDC], a
  ld a, $A5
  ld [wSignedReady], a
.loop
  call WaitVBlank
  call ApplyDisplay
  ld a, [wSignedInspect]
  or a
  jr z, .step
  ; Inspection only: disable LCD at VBlank, copy, then restore the prepared
  ; registers. This pause is not a timing or no-flash guarantee.
  xor a
  ldh [rLCDC], a
  ld hl, wSignedAfter
  call SignedObserve
  call ApplyDisplay
  xor a
  ld [wSignedInspect], a
  ld hl, wSignedDone
  inc [hl]
.step
  ld a, [wSignedFreeze]
  or a
  jr nz, .choose
  call ReadPad
  call Step
.choose
  ; Step may have changed wShown7 at its end while still preparing native
  ; output. Never replace pending field/warning output or defer a scene gate.
  ld a, [wShown7]
  or a
  jr nz, .preempt
  ld a, [wShowMode]
  or a
  jr nz, .preempt
  ld a, [wTraceShow]
  or a
  jr nz, .preempt
  ld a, [wSignedRequest]
  or a
  jr z, .native
  ld a, $81              ; $9800 map, signed tile IDs 0..3 at $9000
  ld [wShLCDC], a
  ld a, 1
  ld [wSignedActive], a
  jp .loop
.native
  ld a, LCDC_NATIVE
  ld [wShLCDC], a
  xor a
  ld [wSignedActive], a
  jp .loop
.preempt
  ld a, [wSignedRequest]
  or a
  jr z, .inactive
  xor a
  ld [wSignedRequest], a
  ld hl, wSignedRefused
  inc [hl]
.inactive
  xor a
  ld [wSignedActive], a
  jp .loop

; Four solid diagnostic stripes, not Lines artwork.
SignedTiles:
  REPT 8
    db $00, $FF          ; colour 2
  ENDR
  REPT 8
    db $FF, $00          ; colour 1
  ENDR
  REPT 8
    db $FF, $FF          ; colour 3
  ENDR
  REPT 8
    db $00, $00          ; colour 0
  ENDR

; Only called with LCD off. HL is the destination.
SignedObserve:
  ld de, $8000
  ld bc, 512
  call SignedCopy
  ld de, $8800
  ld bc, 384
  call SignedCopy
  ld de, $9100
  ld bc, 256
  call SignedCopy
  ld de, $9800
  ld bc, 2048
  jp SignedCopy

SignedCopy:
.loop
  ld a, [de]
  ld [hli], a
  inc de
  dec bc
  ld a, b
  or c
  jr nz, .loop
  ret
