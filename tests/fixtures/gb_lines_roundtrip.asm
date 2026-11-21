; Test-only harness; production entry and dispatch are not changed.
; The host redirects the linked test ROM's entry to RoundtripMain.
; Production owns $C000-$C01A and $C100-$C187. The test mailbox is at
; $C040, the saved unknown VRAM and pending native output at $C200, and
; the 2560-byte VRAM observation buffer at $D000 (DMG fixed WRAM bank).
; The linker must place the relocatable native and Lines state elsewhere.
;
; This harness runs the real ReadPad/Step path even while Lines is visible.
; Consequently NativeStep advances in the background: this is a fixture
; assumption, not a proposed production dispatch or a native cadence test.
; A request while scene 7 or its warning is active is refused, not queued.
; A normal scene-7 entry during Lines triggers restoration before its entry
; step. No saved field, warning visibility, latch, or safety history is
; restored. The real Step owns scene debounce and input history throughout.
; The optional test-only freeze skips ReadPad and Step while continuing to
; apply the prepared display and process the mailbox. It stops native and
; scene-7 clocks and budgets: it is only for matching still-frame captures,
; not a model of production cadence or safety during a handoff.
;
; All VRAM accesses below occur with LCD off, after WaitVBlank. Re-enabling
; the LCD is a fixture operation, not proof of bounded timing or flash safety.
; LinesCopy takes 52 T-cycles per taken iteration, 48 for the last. Entry's
; 192-byte backup, 2560-byte observation and 2560-byte LinesLoad loops alone
; take 276200 T-cycles; restoration's backup and observation loops take at
; least 143092, plus the production loaders. These exclude call/setup costs.

INCLUDE "main.asm"
INCLUDE "lines.asm"

SECTION "Roundtrip mailbox", WRAM0[$C040]
wRoundReady:   ds 1
wRoundRequest: ds 1       ; 1 requests Lines, 0 requests native
wRoundActive:  ds 1
wRoundRefused: ds 1
wRoundLoads:   ds 1
wRoundRestores: ds 1
wRoundHardware: ds 4      ; BGP, LCDC, SCX, SCY read after restoration
wRoundFreeze:  ds 1       ; fixture-only freeze for still-frame comparison

SECTION "Roundtrip backup", WRAM0[$C200]
wRoundUnknown: ds 192     ; original $8040-$80FF
wRoundPending: ds 4       ; BGP, LCDC, SCX, SCY from the latest native step
wRoundTrace:   ds 9       ; pending trace for that same step

SECTION "Roundtrip VRAM observation", WRAMX[$D000], BANK[1]
wRoundVram: ds 512 + 2048

SECTION "Roundtrip code", ROM0
RoundtripMain:
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
  ; Values not supplied by either production loader must survive the trip.
  ld hl, $8040
  ld b, 192
  ld a, $35
.seed
  ld [hli], a
  add 17
  dec b
  jr nz, .seed
  ld a, $E4
  ldh [rBGP], a
  ld [wShBGP], a
  ld a, LCDC_NATIVE
  ld [wShLCDC], a
  ldh [rLCDC], a
  ld a, $A5
  ld [wRoundReady], a

.loop
  call WaitVBlank
  ld a, [wRoundActive]
  or a
  jr z, .request
  ld a, [wRoundRequest]
  or a
  jr z, .restore
  ld a, [wShown7]
  or a
  jr nz, .restore
  call ApplyDisplay
  jr .step
.restore
  call RoundtripRestore
  jr .step
.request
  ld a, [wRoundRequest]
  or a
  jr z, .ordinary
  ld a, [wShowMode]
  cp SHOW_WARN
  jr z, .refuse
  ld a, [wShown7]
  or a
  jr nz, .refuse
  ; The last field output must already have been presented. Only the native
  ; pending output can be displaced and later restored as a native output.
  ld a, [wTraceShow]
  cp SHOW_NATIVE
  jr nz, .refuse
  call RoundtripEnter
  jr .step
.refuse
  xor a
  ld [wRoundRequest], a
  inc a
  ld [wRoundRefused], a
.ordinary
  call ApplyDisplay
.step
  ld a, [wRoundFreeze]
  or a
  jp nz, .loop
  call ReadPad
  call Step
  ld a, [wRoundActive]
  or a
  jr z, .loop
  ; Keep the *prepared* native output, not a redraw from later state.
  call RoundtripSavePending
  ld a, [wShown7]
  or a
  jr nz, .loop           ; restore before running scene 7's entry step
  call RoundtripPrepareLines
  jr .loop

RoundtripEnter:
  xor a
  ldh [rLCDC], a
  call RoundtripSavePending
  ld hl, wRoundUnknown
  ld de, $8040
  ld bc, 192
  call LinesCopy
  call RoundtripObserve
  call LinesLoad
  ld a, 1
  ld [wRoundActive], a
  ld hl, wRoundLoads
  inc [hl]
  call RoundtripPrepareLines
  jp ApplyDisplay        ; LCDC is written last and turns the LCD back on

RoundtripRestore:
  xor a
  ldh [rLCDC], a
  call LoadTiles
  call LoadMap
  call LoadFieldVram
  ld hl, $8040
  ld de, wRoundUnknown
  ld bc, 192
  call LinesCopy
  call RoundtripObserve
  ld hl, wShBGP
  ld de, wRoundPending
  ld bc, 4
  call LinesCopy
  ld hl, wTraceShow
  ld de, wRoundTrace
  ld bc, 9
  call LinesCopy
  xor a
  ld [wRoundActive], a
  ld [wRoundRequest], a
  call ApplyDisplay      ; restore pending registers only after VRAM is ready
  ldh a, [rBGP]
  ld [wRoundHardware], a
  ldh a, [rLCDC]
  ld [wRoundHardware + 1], a
  ldh a, [rSCX]
  ld [wRoundHardware + 2], a
  ldh a, [rSCY]
  ld [wRoundHardware + 3], a
  ld hl, wRoundRestores  ; publish only after registers have been sampled
  inc [hl]
  ret

RoundtripSavePending:
  ld hl, wRoundPending
  ld de, wShBGP
  ld bc, 4
  call RoundtripCopyWram
  ld hl, wRoundTrace
  ld de, wTraceShow
  ld bc, 9
  jp RoundtripCopyWram

; This copy is WRAM-to-WRAM and may run with LCD on.
RoundtripCopyWram:
.loop
  ld a, [de]
  ld [hli], a
  inc de
  dec bc
  ld a, b
  or c
  jr nz, .loop
  ret

RoundtripPrepareLines:
  xor a                  ; deterministic plane request
  ld b, 85
  call LinesStep
  ld a, 3                ; fixture-only trace kind; never pretend warning shown
  ld [wTraceShow], a
  ret

RoundtripObserve:
  ld hl, wRoundVram
  ld de, $8000
  ld bc, 512
  call LinesCopy
  ld de, $9800
  ld bc, 2048
  jp LinesCopy
