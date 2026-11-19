; Colour field (scene 7) for the DMG, following shared/specs/colour-field-v0.md.
;
; Display: one prepared BG map ($9C00) never changes after power-on. Each
; cell holds tile 16 + 4 * row class + column class. With LCDC tile data at
; $8000 the tile colour is its column class; at $8800 it is its row class.
; Vertical layouts use the column classes and horizontal layouts the row
; classes, and BGP maps each class to its region's shade. A step's colours
; and layout are therefore three register writes at VBlank (BGP, LCDC, scroll)
; and a partial layout is never shown.
;
;   column classes (x): 0..47, 48..79, 80..103, 104..159
;   row classes (y):    0..47, 48..71, 72..95,  96..143
;   V2 boundary 80, V3 48 and 104; H2 72 from the bottom, H3 48 and 96.
;
; Input: procedural (chipviz_frame_make_procedural, scene and palette unused)
; until B or A is first held, then the pad as a digital-only USB HID receiver:
; energy and bands are HID_E_LO (the DMG pad has no shoulder), spectrum 0,
; B = beat, A = bar, no notes. Up/Down step the scene ring (0, 7); Left/Right
; step the palette ring.

DEF SIL_AVG      EQU 12
DEF SIL_EXIT     EQU 24
DEF SIL_HOLD     EQU 30
DEF T_FLOOR      EQU 24
DEF T_E_ON       EQU 40
DEF T_E_OFF      EQU 16
DEF T_B_ON       EQU 56
DEF T_B_OFF      EQU 24
DEF FLUX_ON      EQU 200
DEF FLUX_OFF     EQU 64
DEF NOTE_VEL     EQU 64
DEF DOM_MARGIN   EQU 32
DEF DOM_HOLD     EQU 30
DEF SECTION_MIN  EQU 600
DEF CUT_MIN      EQU 20
DEF LEVEL_OPP    EQU 10
DEF GATE_ON      EQU 4
DEF GATE_OFF_MIN EQU 4
DEF REST_DIM_ON  EQU 64
DEF REST_DIM_OFF EQU 48
DEF P_STABLE     EQU 30
DEF HID_E_LO     EQU 64
DEF WARN_STEPS   EQU 600

DEF SHOW_NATIVE  EQU 0
DEF SHOW_FIELD   EQU 1
DEF SHOW_WARN    EQU 2
DEF PIN_AUTO     EQU $FF
DEF NONE         EQU $FF

DEF LCDC_FIELD_V EQU %10011001    ; BG map $9C00, tiles $8000: column classes
DEF LCDC_FIELD_H EQU %10001001    ; BG map $9C00, tiles $8800: row classes
DEF LCDC_WARN    EQU %10001001
DEF BGP_WARN     EQU %00111111    ; colour 3 (text) white, the rest black
DEF SCY_WARN     EQU 144          ; map rows 18..31 hold the warning text

DEF FIXTURE_READY EQU $A5
DEF CTL_WIRE     EQU $01          ; fixture: palette from the wire (else local p)
DEF CTL_PAD      EQU $02          ; fixture: pad as a HID receiver, rings from boot

; Saturating 8-bit increment. 8-bit counters are compared only against
; thresholds below 255, so they behave like the spec's u16 counters.
MACRO INCSAT
  ld hl, \1
  inc [hl]
  jr nz, .incsat\@
  dec [hl]
.incsat\@
ENDM

; hl = \1 + a, for arrays that do not cross a 256-byte page.
MACRO HLIDX
  add LOW(\1)
  ld l, a
  ld h, HIGH(\1)
ENDM

; hl = \1 + a, for ROM tables.
MACRO HLROM
  add LOW(\1)
  ld l, a
  adc HIGH(\1)
  sub l
  ld h, a
ENDM

; ---------------------------------------------------------------------------
; Trace and test mailbox at fixed addresses, read by
; scripts/gb_libretro_capture.py.
SECTION "Trace", WRAM0[$C000]
wTraceShow: ds 1      ; $C000 0 native, 1 colour field, 2 warning
wTrace:     ds 8      ; $C001 trace of the rendered step: mode, family,
                      ;       layout, combo, rot, lvl0..2 (before requests)
wShownShow: ds 1      ; $C009 the same nine bytes, as written at VBlank
wShown:     ds 8
wStep:      ds 2      ; $C012
wAckSeq:    ds 1      ; $C014 fixture: last completed frame counter
wReady:     ds 1      ; $C015 fixture: FIXTURE_READY once initialised
wShBGP:     ds 1      ; $C016 display registers for the next VBlank
wShLCDC:    ds 1
wShSCX:     ds 1
wShSCY:     ds 1
wLastSeq:   ds 1

IF DEF(CHIPVIZ_FIXTURE)
SECTION "Fixture mailbox", WRAM0[$C020]
wFxSeq:     ds 1      ; $C020 frame counter; a new value runs one step
wFxCtl:     ds 1      ; $C021 CTL_* bits, read at boot
wFxLocalP:  ds 1      ; $C022 local p (setup) when CTL_WIRE is clear
wFxArmed:   ds 1      ; $C023 local p 3 or 7 is pre-armed
wFxPin:     ds 1      ; $C024 pin at power-on (PIN_AUTO or 0..3)
wFxScene:   ds 1      ; $C025 frame scene
wFxPal:     ds 1      ; $C026 frame palette
wFxOpReq:   ds 1      ; $C027 operator pulse this step
wFxOpP:     ds 1      ; $C028
wFxOpArmed: ds 1      ; $C029
wFxIn:      ds 22     ; $C02A e, bass, mid, treble, spectrum[8], flags,
                      ;       note_count, notes[4] as (note, velocity)
ENDC

; All colour-field state in one page so HLIDX can index it.
SECTION "Colour field state", WRAM0[$C100]
wMode:      ds 1      ; live state in trace order (SnapTrace copies it)
wFamily:    ds 1
wLayout:    ds 1
wCombo:     ds 1
wRot:       ds 1
wDisp:      ds 3
wIn:        ds 4      ; e, bass, mid, treble
wSpec:      ds 8
wFlags:     ds 1
wNoteN:     ds 1
wNotes:     ds 8
wPrevFlags: ds 1
wPrevNoteN: ds 1
wPrevNotes: ds 8
wPrevSpec:  ds 8
wAcc:       ds 8      ; u16 EMA accumulators, little endian
wDPos:      ds 4      ; max(0, x - avg_x) before the update
wAvgB:      ds 3
wFluxLo:    ds 1
wFluxHi:    ds 1
wLvl:       ds 3
wShade:     ds 3
wTArmed:    ds 1
wSilent:    ds 1
wQuiet:     ds 1
wDom:       ds 1
wCand:      ds 1
wCandN:     ds 1
wEvT:       ds 1
wEvN:       ds 1
wEvB:       ds 1
wEvX:       ds 1
wEvE:       ds 1
wEvD:       ds 1
wAct:       ds 1
wPulse:     ds 1
wHyst:      ds 1
wDim:       ds 1
wSinceCut:  ds 1
wSinceLay:  ds 2
wGateOff:   ds 1
wSincePulse: ds 1
wLg:        ds 1
wPEff:      ds 1
wLatch:     ds 1      ; flash latch
wPin:       ds 1
wPinReq:    ds 1
wPinPending: ds 1
wReqPending: ds 1
wReqP:      ds 1
wReqArmed:  ds 1
wReqWire:   ds 1      ; the pending request came from the wire gate
wShowMode:  ds 1
wShown7:    ds 1
wEntryPending: ds 1
wPowerOn:   ds 1
wBooted:    ds 1
wDebVal:    ds 1
wDebN:      ds 1
wSinceScene: ds 1     ; steps since a scene-7 entry or exit took effect
wSceneIn:   ds 1      ; incoming scene this step
wPalIn:     ds 1      ; incoming palette p = palette & 7
wInAbs:     ds 1      ; scene and palette were carried absolutely
wWireVal:   ds 1
wWireN:     ds 1
wWireUsed:  ds 1
wWireAbs:   ds 1      ; the current run began with an absolute value
wSceneRing: ds 1
wPalRing:   ds 1
wPad:       ds 1
wPadNew:    ds 1
wPadSource: ds 1      ; nonzero once B or A was held: input is the pad
wProcLo:    ds 1
wProcHi:    ds 1
wProcPhase: ds 1      ; frame * 17
wProcMid:   ds 1      ; frame * 9
wProcTreble: ds 1     ; frame * 23
wProcNote:  ds 1      ; (frame / 16) % 24
wPalSel:    ds 1      ; operator: local palette
wOpPal:     ds 1
wOpPin:     ds 1
wWarnP:     ds 1
wWarnLo:    ds 1
wWarnHi:    ds 1
wN:         ds 1
wStateEnd:
ASSERT wStateEnd <= $C200

; ---------------------------------------------------------------------------
SECTION "Colour field", ROM0

InitState:
  ld a, PIN_AUTO
  ld [wPin], a
  ld [wPinReq], a
  ld [wCand], a
  ld a, CUT_MIN          ; no previous scene change bounds the first one
  ld [wSinceScene], a
  ret

; One step: input, rings, field logic, output, then the scene-7 debounce.
Step:
  ld hl, wStep
  inc [hl]
  jr nz, .boot
  inc hl
  inc [hl]
.boot
IF DEF(CHIPVIZ_FIXTURE)
  ld a, [wBooted]
  or a
  call z, FixtureBoot
ENDC
  call MakeInput
IF DEF(CHIPVIZ_INPUT_ROLE_OPERATOR)
  ld a, [wShowMode]
  cp SHOW_WARN
  jp z, WarnStep
ENDC
  call UpdateRings
  ld a, [wBooted]
  or a
  jr nz, .run
  inc a
  ld [wBooted], a
  ld a, [wSceneIn]
  ld [wDebVal], a
  cp 7
  jr nz, .run
  ld a, 1                ; power-on in scene 7: entry on the first step
  ld [wShown7], a
  ld [wEntryPending], a
  ld [wPowerOn], a
  xor a
  ld [wSinceScene], a
.run
  ld a, [wShown7]
  or a
  jr z, .output
  ld a, [wEntryPending]
  or a
  jr z, .field
  xor a
  ld [wEntryPending], a
  call SceneEntry
  call BuildField
IF DEF(CHIPVIZ_INPUT_ROLE_OPERATOR)
  call EntryPulse
ENDC
  jr .debounce
.field
  ; Steps 1-7 and the render, then requests: a request applied at the end
  ; of step t is visible from t + 1.
  call FieldStep
  call BuildField
  call UpdateRequests
  jr .debounce
.output
  ld a, [wShowMode]
  cp SHOW_FIELD
  jr nz, .notField
  call BuildField
  jr .debounce
.notField
IF DEF(CHIPVIZ_INPUT_ROLE_OPERATOR)
  cp SHOW_WARN
  jr nz, .native
  call ShowWarning
  jr .debounce
.native
ENDC
  call NativeStep
.debounce
  jp SceneDebounce

IF DEF(CHIPVIZ_FIXTURE)
FixtureBoot:
  ld a, [wFxPin]
  ld [wPin], a
  ld [wPinReq], a
  ld a, [wFxScene]
  ld [wSceneRing], a
  ld a, [wFxPal]
  and 7
  ld [wPalRing], a
  ld a, [wFxCtl]
  and CTL_PAD
  ld [wPadSource], a
  ret
ENDC

; ---------------------------------------------------------------------------
; Step input.
MakeInput:
IF DEF(CHIPVIZ_FIXTURE)
  ld a, [wFxCtl]
  and CTL_PAD
  jr nz, .live
  ld hl, wFxIn
  ld de, wIn
  ld c, 22
.copy
  ld a, [hli]
  ld [de], a
  inc de
  dec c
  jr nz, .copy
  ld a, [wFxScene]
  ld [wSceneIn], a
  ld a, [wFxPal]
  and 7
  ld [wPalIn], a
  ld a, 1
  ld [wInAbs], a
  ret
.live
ENDC
  ld a, [wPadSource]
  or a
  jr nz, .pad
  ld a, [wPad]
  and PAD_A | PAD_B
  jr z, ProcInput
  ld [wPadSource], a
.pad
  ld hl, wIn
  ld a, HID_E_LO
  ld [hli], a
  ld [hli], a
  ld [hli], a
  ld [hli], a
  xor a
  ld c, 8
.spec
  ld [hli], a
  dec c
  jr nz, .spec
  ld b, 0
  ld a, [wPad]
  and PAD_B
  jr z, .bar
  set 0, b               ; B (South) = beat
.bar
  ld a, [wPad]
  and PAD_A
  jr z, .flags
  set 1, b               ; A (East) = bar
.flags
  ld a, b
  ld [wFlags], a
  xor a
  ld [wNoteN], a
  ret

ProcInput:
  ld a, [wProcPhase]
  ld b, a
  srl a
  add 80
  ld [wIn], a            ; energy = 80 + phase / 2
  ld a, b
  cpl
  ld [wIn + 1], a        ; bass = 255 - phase
  ld a, [wProcMid]
  and $7F
  add 64
  ld [wIn + 2], a
  ld a, [wProcTreble]
  ld [wIn + 3], a
  ld hl, wSpec
  ld a, b
  ld c, 8
.spec
  ld [hli], a
  add 29
  dec c
  jr nz, .spec
  xor a
  ld [wFlags], a
  ld [wNoteN], a
  ld a, b
  cp 17
  jr nc, .advance
  ld a, 1
  ld [wFlags], a
  ld [wNoteN], a
  ld a, [wProcNote]
  add 48
  ld [wNotes], a
  ld a, [wIn]
  ld [wNotes + 1], a
.advance
  ld a, b
  add 17
  ld [wProcPhase], a
  ld a, [wProcMid]
  add 9
  ld [wProcMid], a
  ld a, [wProcTreble]
  add 23
  ld [wProcTreble], a
  ld hl, wProcLo
  inc [hl]
  jr nz, .beat
  inc hl
  inc [hl]
  jr nz, .beat
  xor a                  ; frame index wrapped to 0
  ld [wProcNote], a
  ret
.beat
  ld a, [wProcLo]
  and $0F
  ret nz
  ld a, [wProcNote]
  inc a
  cp 24
  jr c, .note
  xor a
.note
  ld [wProcNote], a
  ret

; Scene ring (0, 7) on Up/Down, palette ring on Right/Left, pin on Select
; (operator role). Pressed edges only; opposing held directions cancel.
UpdateRings:
IF DEF(CHIPVIZ_FIXTURE)
  ld a, [wFxCtl]
  and CTL_PAD
  ret z
ENDC
  ld a, [wPad]
  and PAD_UP | PAD_DOWN
  cp PAD_UP | PAD_DOWN
  jr z, .palette
  ld a, [wPadNew]
  and PAD_UP | PAD_DOWN
  jr z, .palette
  ld a, [wSceneRing]
  xor 7
  ld [wSceneRing], a
.palette
  ld a, [wPad]
  and PAD_LEFT | PAD_RIGHT
  cp PAD_LEFT | PAD_RIGHT
  jr z, .pin
  ld a, [wPadNew]
  and PAD_RIGHT
  jr z, .left
  ld a, 1
  jr .apply
.left
  ld a, [wPadNew]
  and PAD_LEFT
  jr z, .pin
  ld a, $FF
.apply
IF DEF(CHIPVIZ_INPUT_ROLE_OPERATOR)
  ld hl, wPalSel
  add [hl]
  and 7
  ld [hl], a
  ld a, [wShown7]        ; outside scene 7 nothing is shown: no pulse
  or a
  jr z, .pin
  ld [wOpPal], a
ELSE
  ld hl, wPalRing
  add [hl]
  and 7
  ld [hl], a
ENDC
.pin
IF DEF(CHIPVIZ_INPUT_ROLE_OPERATOR)
  ld a, [wPadNew]
  and PAD_SELECT
  jr z, .done
  ld a, [wPinReq]        ; auto -> V2 -> H2 -> V3 -> H3 -> auto
  inc a
  cp 4
  jr nz, .setPin
  ld a, PIN_AUTO
.setPin
  ld [wPinReq], a
  ld a, 1
  ld [wOpPin], a
  ld a, [wShown7]
  or a
  jr z, .pinNow
  ld a, [wEntryPending]  ; entering this step: the entry uses wPin
  or a
  jr z, .done
.pinNow
  ld a, [wPinReq]        ; nothing visible yet: apply now
  ld [wPin], a
  xor a
  ld [wOpPin], a
.done
ENDC
  ld a, [wSceneRing]
  ld [wSceneIn], a
  ld a, [wPalRing]
  ld [wPalIn], a
  xor a                  ; ring values are relative, never absolute
  ld [wInAbs], a
  ret

; Scene-7 debounce at the end of the step. Data role: a change needs the
; incoming scene unchanged for P_STABLE steps, and at least CUT_MIN steps
; since the last entry or exit. Operator role: local Up/Down scene inputs are
; exempt and take effect at once. The DMG has no wire or link-cable scene
; source today, so the floor never binds (P_STABLE > CUT_MIN). A future
; debounced source on the operator build must take the gated path below, so
; that wSinceScene spaces it after an exempt local change.
SceneDebounce:
  INCSAT wSinceScene
  ld a, [wSceneIn]
  ld b, a
  ld a, [wDebVal]
  cp b
  jr z, .same
  ld a, b
  ld [wDebVal], a
  xor a
  ld [wDebN], a
.same
  INCSAT wDebN
  ld c, 0
  ld a, b
  cp 7
  jr nz, .want
  inc c
.want
  ld a, [wShown7]
  cp c
  ret z
IF !DEF(CHIPVIZ_INPUT_ROLE_OPERATOR)
  ld a, [wDebN]
  cp P_STABLE
  ret c
  ld a, [wSinceScene]
  cp CUT_MIN
  ret c
ENDC
  xor a
  ld [wSinceScene], a
  ld a, c
  ld [wShown7], a
  or a
  jr z, .exit
  ld [wEntryPending], a
  ret
.exit
  xor a                  ; native from the next step; the latch clears
  ld [wShowMode], a
  ld [wLatch], a
  ld [wOpPal], a
  ld [wOpPin], a
  ret

; ---------------------------------------------------------------------------
; Entry step: initialise, render, no events.
SceneEntry:
  ld hl, wAcc
  ld de, wIn
  ld c, 4
.acc
  ld a, [de]             ; acc = x << 3
  inc de
  ld b, a
  add a
  add a
  add a
  ld [hli], a
  ld a, b
  swap a
  srl a
  and 7
  ld [hli], a
  dec c
  jr nz, .acc
  ld hl, wSpec
  ld de, wPrevSpec
  ld c, 8
.spec
  ld a, [hli]
  ld [de], a
  inc de
  dec c
  jr nz, .spec
  call SavePrevInput
  ld a, 1
  ld [wTArmed], a
  xor a
  ld [wSilent], a
  ld [wQuiet], a
  ld [wCandN], a
  ld [wRot], a
  ld [wCombo], a
  ld [wLatch], a
  ld [wReqPending], a
  ld [wPinPending], a
  ld [wOpPin], a
  ld [wSinceLay], a
  ld [wSinceLay + 1], a
  ld a, NONE
  ld [wCand], a
  ; dom = argmax of the raw bands, ties to the lowest index
  ld c, 0
  ld a, [wIn + 1]
  ld b, a
  ld a, [wIn + 2]
  cp b
  jr c, .dom2
  jr z, .dom2
  ld c, 1
  ld b, a
.dom2
  ld a, [wIn + 3]
  cp b
  jr c, .domSet
  jr z, .domSet
  ld c, 2
.domSet
  ld a, c
  ld [wDom], a
  ld a, [wPin]
  ld [wPinReq], a
  cp PIN_AUTO
  jr nz, .layout
  xor a
.layout
  ld [wLayout], a
IF DEF(CHIPVIZ_INPUT_ROLE_OPERATOR)
  ld a, [wPalSel]
ELSE
IF DEF(CHIPVIZ_FIXTURE)
  ld a, [wFxCtl]
  and CTL_WIRE | CTL_PAD
  jr nz, .wire
  ld a, [wFxArmed]       ; setup: local p, pre-armed by the operator harness
  ld [wLatch], a
  ld a, [wFxLocalP]
  jr .peff
.wire
ENDC
  ld a, [wPalIn]         ; wire: the entry value counts as step 1, but
  ld [wWireVal], a       ; only inside the scene-7 window
  ld a, [wSceneIn]
  cp 7
  ld a, 0
  jr nz, .wireN
  inc a
.wireN
  ld [wWireN], a
  xor a
  ld [wWireUsed], a
  ld a, [wInAbs]
  ld [wWireAbs], a
  ld a, [wPalIn]
ENDC
.peff
  and 7
  ld [wPEff], a
  call SetModeFromPEff   ; unarmed: a 3 or 7 runs as 2 or 6
  ld a, GATE_ON + GATE_OFF_MIN
  ld [wSincePulse], a
  call ModeEntry
  ld a, [wPowerOn]
  or a
  jr z, .shown
  xor a                  ; power-on entry: no budget used
  ld [wPowerOn], a
  ld a, CUT_MIN
  ld [wSinceCut], a
  ld a, LEVEL_OPP
  ld [wLg], a
.shown
  ld a, SHOW_FIELD
  ld [wShowMode], a
  ret

; Mode and family from p_eff and the latch (3 without the latch runs as 2).
SetModeFromPEff:
  ld a, [wPEff]
  srl a
  srl a
  ld [wFamily], a
  ld a, [wPEff]
  and 3
  cp 3
  jr nz, .set
  ld a, [wLatch]
  or a
  ld a, 3
  jr nz, .set
  ld a, 2
.set
  ld [wMode], a
  ret

; Mode entry: on scene entry and on every mode or family change.
ModeEntry:
  xor a
  ld [wAct], a
  ld [wSinceCut], a
  ld [wLg], a
  ld [wPulse], a
  ld a, GATE_OFF_MIN
  ld [wGateOff], a
  ld a, [wMode]
  or a
  jr nz, .levels
  ld a, [wDom]
  ld [wCombo], a
  ld a, [wMode]
.levels
  cp 1
  call z, InitLevels
  ld c, 0
  call GetAvg
  ld b, 0
  cp REST_DIM_ON
  jr c, .dim
  inc b
.dim
  ld a, b
  ld [wHyst], a
  ld [wDim], a
  ld a, [wSilent]
  or a
  ret z
  ld a, [wMode]
  and 1                  ; spectral (1) or flash/gate (3)
  ret z
  xor a
  ld [wLvl], a
  ld [wLvl + 1], a
  ld [wLvl + 2], a
  ld [wHyst], a
  ld [wDim], a
  ret

SavePrevInput:
  ld hl, wFlags
  ld de, wPrevFlags
  ld c, 10
.copy
  ld a, [hli]
  ld [de], a
  inc de
  dec c
  jr nz, .copy
  ret

; a = avg of input c (acc >> 3). Clobbers hl.
GetAvg:
  ld a, c
  add a
  HLIDX wAcc
  ld a, [hli]
  ld h, [hl]
  ld l, a
  srl h
  rr l
  srl h
  rr l
  srl h
  rr l
  ld a, l
  ret

; a = region count (2 or 3).
Regions:
  ld a, [wLayout]
  srl a
  add 2
  ret

; a = v_c for region c: bass, mid, treble (3 regions) or bass,
; max(mid, treble) (2 regions). Keeps b, c, d, e.
RegionValue:
  ld a, [wLayout]
  and 2
  jr nz, .raw
  ld a, c
  cp 1
  jr nz, .raw
  ld a, [wIn + 3]
  ld l, a
  ld a, [wIn + 2]
  cp l
  ret nc
  ld a, l
  ret
.raw
  ld a, c
  inc a
  HLIDX wIn
  ld a, [hl]
  ret

; lvl[i] = count(UP[k] <= v_i); lvl[2] = 0 in 2-region layouts.
InitLevels:
  xor a
  ld [wLvl + 2], a
  call Regions
  ld [wN], a
  ld c, 0
.loop
  call RegionValue
  ld b, 0
  cp 48
  jr c, .set
  inc b
  cp 104
  jr c, .set
  inc b
  cp 168
  jr c, .set
  inc b
.set
  ld a, c
  HLIDX wLvl
  ld [hl], b
  inc c
  ld a, [wN]
  cp c
  jr nz, .loop
  ret

; Layout operations.
NextLayout:
  ld a, [wPin]
  cp PIN_AUTO
  ret nz                 ; pinned: event-driven next_layout does nothing
  ld a, [wLayout]
  inc a
  and 3
SetLayout:
  ld [wLayout], a
  call Regions
  ld b, a
  ld a, [wRot]
  cp b
  jr c, .kept
  xor a                  ; rot = rot mod n (rot <= 2)
  ld [wRot], a
.kept
  xor a
  ld [wSinceLay], a
  ld [wSinceLay + 1], a
  ld [wLg], a
  ld a, [wMode]
  cp 1
  ret nz
  jp InitLevels

NextCombo:
  ld a, [wCombo]
  inc a
  and 3
  ld [wCombo], a
  ret

Rotate:
  call Regions
  ld b, a
  ld a, [wRot]
  inc a
  cp b
  jr c, .set
  xor a
.set
  ld [wRot], a
  ret

; ---------------------------------------------------------------------------
; Steps 1-7 of every later step.
FieldStep:
  ; 1. Deltas against the previous averages, then the EMA update.
  ld c, 3
.ema
  call GetAvg
  ld b, a                ; old average
  ld a, c
  HLIDX wIn
  ld d, [hl]             ; x
  ld a, d
  sub b
  jr nc, .delta
  xor a
.delta
  ld e, a
  ld a, c
  HLIDX wDPos
  ld [hl], e
  ld a, c
  add a
  HLIDX wAcc
  ld a, [hl]             ; acc += x - (acc >> 3)
  sub b
  ld [hli], a
  ld a, [hl]
  sbc 0
  ld [hld], a
  ld a, [hl]
  add d
  ld [hli], a
  ld a, [hl]
  adc 0
  ld [hl], a
  dec c
  bit 7, c
  jr z, .ema
  xor a
  ld [wFluxLo], a
  ld [wFluxHi], a
  ld c, 0
.flux
  ld a, c
  HLIDX wSpec
  ld b, [hl]
  ld a, c
  HLIDX wPrevSpec
  ld a, b
  sub [hl]
  ld [hl], b
  jr c, .fluxNext
  ld hl, wFluxLo
  add [hl]
  ld [hli], a
  jr nc, .fluxNext
  inc [hl]
.fluxNext
  inc c
  ld a, c
  cp 8
  jr nz, .flux

  ; 2. Silence.
  ld a, [wSilent]
  or a
  jr z, .quiet
  ld a, [wIn]
  cp SIL_EXIT
  jr c, .blackout
  xor a
  ld [wSilent], a
  ld [wQuiet], a
  jr .blackout
.quiet
  ld c, 0
  call GetAvg
  cp SIL_AVG + 1
  jr nc, .loud
  INCSAT wQuiet
  jr .silCheck
.loud
  xor a
  ld [wQuiet], a
.silCheck
  ld a, [wQuiet]
  cp SIL_HOLD
  jr c, .blackout
  ld a, 1
  ld [wSilent], a
  xor a
  ld [wDim], a
  ld a, [wPulse]
  or a
  jr z, .blackout
  xor a                  ; a lit pulse ends early
  ld [wPulse], a
  ld [wGateOff], a
.blackout
  ld a, [wMode]
  cp 1
  jr nz, .transients
  ld a, [wSilent]
  or a
  jr z, .transients
  ld a, [wLvl]
  ld hl, wLvl + 1
  or [hl]
  inc hl
  or [hl]
  jr z, .transients
  ld a, [wLg]
  cp LEVEL_OPP
  jr c, .transients
  xor a
  ld [wLvl], a
  ld [wLvl + 1], a
  ld [wLvl + 2], a
  ld [wLg], a

  ; 3. Transients. hit and calm are mutually exclusive.
.transients
  xor a
  ld [wEvT], a
  ld a, [wIn]
  cp T_FLOOR
  jr c, .calm
  ld a, [wDPos]
  cp T_E_ON
  jr nc, .hit
  ld a, [wDPos + 1]
  cp T_B_ON
  jr nc, .hit
  ld a, [wDPos + 2]
  cp T_B_ON
  jr nc, .hit
  ld a, [wDPos + 3]
  cp T_B_ON
  jr nc, .hit
  ld a, [wFluxHi]
  or a
  jr nz, .hit
  ld a, [wFluxLo]
  cp FLUX_ON
  jr c, .calm
.hit
  ld a, [wTArmed]
  or a
  jr z, .events
  xor a
  ld [wTArmed], a
  inc a
  ld [wEvT], a
  jr .events
.calm
  ld a, [wDPos]
  cp T_E_OFF
  jr nc, .events
  ld a, [wDPos + 1]
  cp T_B_OFF
  jr nc, .events
  ld a, [wDPos + 2]
  cp T_B_OFF
  jr nc, .events
  ld a, [wDPos + 3]
  cp T_B_OFF
  jr nc, .events
  ld a, [wFluxHi]
  or a
  jr nz, .events
  ld a, [wFluxLo]
  cp FLUX_OFF
  jr nc, .events
  ld a, 1
  ld [wTArmed], a

  ; 4. Events.
.events
  call NoteOnset
  ld [wEvN], a
  ld a, [wPrevFlags]
  cpl
  ld b, a
  ld a, [wFlags]
  and b
  ld b, a
  and 3                  ; beat or bar
  ld [wEvB], a
  ld a, b
  and 4                  ; drop
  ld [wEvX], a
  call SavePrevInput
  ld b, 0
  ld a, [wSilent]
  or a
  jr nz, .setE
  ld a, [wEvT]
  ld hl, wEvN
  or [hl]
  ld hl, wEvB
  or [hl]
  ld hl, wEvX
  or [hl]
  ld b, a
.setE
  ld a, b
  ld [wEvE], a

  ; 5. Dominance on the new averages.
  ld c, 1
  call GetAvg
  ld [wAvgB], a
  ld c, 2
  call GetAvg
  ld [wAvgB + 1], a
  ld c, 3
  call GetAvg
  ld [wAvgB + 2], a
  ld c, 0
  ld a, [wAvgB]
  ld b, a
  ld a, [wAvgB + 1]
  cp b
  jr c, .c2
  jr z, .c2
  ld c, 1
  ld b, a
.c2
  ld a, [wAvgB + 2]
  cp b
  jr c, .domC
  jr z, .domC
  ld c, 2
.domC
  xor a
  ld [wEvD], a
  ld a, [wSilent]
  or a
  jr nz, .domReset
  ld a, [wDom]
  cp c
  jr z, .domReset
  HLIDX wAvgB
  ld a, [hl]
  add DOM_MARGIN
  jr c, .domReset
  ld b, a
  ld a, c
  HLIDX wAvgB
  ld a, [hl]
  cp b
  jr c, .domReset
  ld a, [wCand]
  cp c
  jr z, .candInc
  ld a, c
  ld [wCand], a
  ld a, 1
  ld [wCandN], a
  jr .domCheck
.candInc
  INCSAT wCandN
.domCheck
  ld a, [wCandN]
  cp DOM_HOLD
  jr c, .rule
  ld a, c
  ld [wDom], a
  xor a
  ld [wCandN], a
  inc a
  ld [wEvD], a
  jr .rule
.domReset
  ld a, NONE
  ld [wCand], a
  xor a
  ld [wCandN], a

  ; 6. Mode rule, unless silent.
.rule
  ld a, [wSilent]
  or a
  jr nz, .counters
  ld a, [wMode]
  or a
  jr nz, .rule1
  call HeldRule
  jr .counters
.rule1
  cp 1
  jr nz, .rule2
  call SpectralRule
  jr .counters
.rule2
  cp 2
  jr nz, .rule3
  call CutsRule
  jr .counters
.rule3
  call GateRule

  ; 7. Counters, then requests under the change budget.
.counters
  INCSAT wSinceCut
  INCSAT wGateOff
  INCSAT wSincePulse
  INCSAT wLg
  ld hl, wSinceLay
  inc [hl]
  jr nz, .requests
  inc hl
  inc [hl]
  jr nz, .requests
  ld a, $FF
  ld [hld], a
  ld [hl], a
.requests
  ret

; a = 1 when a (note, velocity) with velocity >= NOTE_VEL is not in the
; previous frame's notes.
NoteOnset:
  ld a, [wNoteN]
  cp 5
  jr c, .count
  ld a, 4
.count
  or a
  ret z
  ld b, a
  ld hl, wNotes
.loop
  ld d, [hl]
  inc hl
  ld e, [hl]
  inc hl
  ld a, e
  cp NOTE_VEL
  jr c, .next
  push hl
  push bc
  ld a, [wPrevNoteN]
  cp 5
  jr c, .prevCount
  ld a, 4
.prevCount
  ld c, a
  ld hl, wPrevNotes
  or a
  jr z, .new
.search
  ld a, [hli]
  cp d
  jr nz, .skip
  ld a, [hl]
  cp e
  jr z, .old
.skip
  inc hl
  dec c
  jr nz, .search
.new
  pop bc
  pop hl
  ld a, 1
  ret
.old
  pop bc
  pop hl
.next
  dec b
  jr nz, .loop
  xor a
  ret

; carry = since_layout >= SECTION_MIN
SinceLayoutLong:
  ld a, [wSinceLay]
  sub LOW(SECTION_MIN)
  ld a, [wSinceLay + 1]
  sbc HIGH(SECTION_MIN)
  ccf
  ret

; carry = SECTION = X || (D && since_layout >= SECTION_MIN)
IsSection:
  ld a, [wEvX]
  or a
  jr nz, .yes
  ld a, [wEvD]
  or a
  ret z
  jr SinceLayoutLong
.yes
  scf
  ret

HeldRule:
  ld a, [wEvD]
  or a
  jr z, .section
  ld a, [wDom]
  ld [wCombo], a
  xor a
  ld [wLg], a
.section
  call IsSection
  ret nc
  ld a, [wSinceCut]
  cp CUT_MIN
  ret c
  call NextLayout
  xor a
  ld [wSinceCut], a
  ret

SpectralRule:
  ld a, [wLg]
  cp LEVEL_OPP
  ret c                  ; X and D are dropped, and levels wait
  ld a, [wEvX]
  or a
  jr z, .dom
  ld a, [wSinceCut]
  cp CUT_MIN
  jr c, .dom
  call NextLayout
  call NextCombo
  xor a
  ld [wSinceCut], a
  ld [wLg], a
  ret
.dom
  ld a, [wEvD]
  or a
  jr z, .levels
  call SinceLayoutLong
  jr nc, .levels
  ld a, [wPin]
  cp PIN_AUTO
  jr nz, .levels
  jp NextLayout          ; sets lg = 0, which blocks the level update
.levels
  call Regions
  ld [wN], a
  ld c, 0
  ld e, 0
.loop
  call RegionValue
  ld d, a
  ld a, c
  HLIDX wLvl
  ld b, [hl]
  ld a, b
  cp 3
  jr nc, .down
  push hl
  ld a, b
  HLROM UpTab + 1
  ld a, d
  cp [hl]
  pop hl
  jr c, .down
  inc [hl]
  ld e, 1
  jr .next
.down
  ld a, b
  or a
  jr z, .next
  push hl
  HLROM DownTab
  ld a, d
  cp [hl]
  pop hl
  jr nc, .next
  dec [hl]
  ld e, 1
.next
  inc c
  ld a, [wN]
  cp c
  jr nz, .loop
  ld a, e
  or a
  ret z
  xor a
  ld [wLg], a
  ret

CutsRule:
  ld a, [wEvE]
  or a
  ret z
  ld a, [wSinceCut]
  cp CUT_MIN
  ret c
  xor a
  ld [wSinceCut], a
  ld [wLg], a
  ld a, [wEvX]
  or a
  jr z, .act
  call NextLayout
  jp NextCombo
.act
  ld a, [wAct]
  ld b, a
  inc a
  and 3
  ld [wAct], a
  ld a, b
  bit 0, a
  jp z, Rotate           ; actions 0 and 2
  cp 1
  jp z, NextCombo
  ld a, [wPin]
  cp PIN_AUTO
  jp nz, Rotate          ; pinned: action 3 rotates instead
  jp NextLayout

GateRule:
  ld c, 0
  call GetAvg
  cp REST_DIM_ON
  jr c, .hystOff
  ld a, 1
  ld [wHyst], a
  jr .pulse
.hystOff
  cp REST_DIM_OFF
  jr nc, .pulse
  xor a
  ld [wHyst], a
.pulse
  ld a, [wPulse]
  or a
  jr z, .start
  dec a
  ld [wPulse], a
  ret nz
  xor a
  ld [wGateOff], a
  ld a, [wHyst]
  ld [wDim], a
  ret
.start
  ld a, [wEvE]
  or a
  ret z
  ld a, [wGateOff]
  cp GATE_OFF_MIN
  ret c
  ld a, [wSincePulse]
  cp GATE_ON + GATE_OFF_MIN
  ret c
  ld a, GATE_ON
  ld [wPulse], a
  xor a
  ld [wSincePulse], a
  call Rotate
  ld a, [wEvX]
  or a
  ret z
  jp NextCombo

; ---------------------------------------------------------------------------
; Step 7 requests: operator pulses or the wire gate, pin, then the budget.
UpdateRequests:
IF DEF(CHIPVIZ_INPUT_ROLE_OPERATOR)
  ld a, [wOpPal]
  or a
  jr z, .pinPulse
  xor a
  ld [wOpPal], a
  ld a, [wPalSel]
  and 3
  cp 3
  jr nz, .operator
  ld a, [wLatch]
  or a
  jr nz, .operator
  ld a, [wPalSel]        ; unarmed mode 3: show the warning screen
  jp StartWarning
.operator
  ld a, [wPalSel]
  ld b, a
  ld a, [wLatch]         ; already armed: a 3 or 7 stays armed
  ld c, a
  xor a
  call SetRequest
.pinPulse
  ld a, [wOpPin]
  or a
  jr z, ReqClearEqual
  xor a
  ld [wOpPin], a
  inc a
  ld [wPinPending], a
ELSE
IF DEF(CHIPVIZ_FIXTURE)
  ld a, [wFxOpReq]
  or a
  jr z, .wireGate
  ld a, 1                ; an operator pulse consumes the current wire value
  ld [wWireUsed], a
  ld a, [wFxOpP]
  and 7
  ld b, a
  ld a, [wFxOpArmed]
  ld c, a
  xor a
  call SetRequest
.wireGate
  ld a, [wFxCtl]
  and CTL_WIRE | CTL_PAD
  jr z, ReqClearEqual    ; local p: the frame's palette is ignored
ENDC
  ; Wire gate with the scene-7 window.
  ld a, [wSceneIn]
  cp 7
  jr z, .count
  xor a                  ; outside the window: restart, drop a wire request
  ld [wWireN], a
  ld a, [wReqWire]
  or a
  jr z, ReqClearEqual
  xor a
  ld [wReqPending], a
  ld [wReqWire], a
  jr ReqClearEqual
.count
  ld a, [wPalIn]
  ld b, a
  ld a, [wWireVal]
  cp b
  jr z, .same
  ld a, b
  ld [wWireVal], a
  xor a
  ld [wWireN], a
  ld [wWireUsed], a
  ld a, [wInAbs]         ; a run keeps the source it began with
  ld [wWireAbs], a
.same
  INCSAT wWireN
  ld a, [wWireN]
  cp P_STABLE
  jr nz, ReqClearEqual
  ld a, [wWireUsed]
  or a
  jr nz, ReqClearEqual
  inc a
  ld [wWireUsed], a
  ld a, [wWireVal]
  ld b, a
  ld a, [wWireAbs]       ; relative runs never arm
  ld c, a
  ld a, 1
  call SetRequest
ENDC
ReqClearEqual:
  ; A request equal in effective mode and family is cleared without effect.
  ld a, [wReqPending]
  or a
  jr z, .pinFree
  call RequestMode
  ld b, a
  ld a, [wMode]
  cp b
  jr nz, .pinFree
  ld a, [wFamily]
  cp e
  jr nz, .pinFree
  xor a
  ld [wReqPending], a
.pinFree
  ; Setting auto, or pinning the shown layout, needs no budget.
  ld a, [wPinPending]
  or a
  jr z, .budget
  ld a, [wPinReq]
  cp PIN_AUTO
  jr z, .pinApplyFree
  ld b, a
  ld a, [wLayout]
  cp b
  jr nz, .budget
  ld a, b
.pinApplyFree
  ld [wPin], a
  xor a
  ld [wPinPending], a
.budget
  ld a, [wLg]
  cp LEVEL_OPP
  ret c
  ld a, [wSinceCut]
  cp CUT_MIN
  ret c
  ld a, [wReqPending]
  or a
  jr z, .pin
  xor a
  ld [wReqPending], a
  ld a, [wReqP]
  ld [wPEff], a
  call RequestMode
  ld [wMode], a
  ld b, a
  ld a, e
  ld [wFamily], a
  ld a, b
  cp 3
  ld a, 0
  jr nz, .latch
  inc a                  ; mode 3 here means the request was armed
.latch
  ld [wLatch], a
  jp ModeEntry
.pin
  ld a, [wPinPending]
  or a
  ret z
  xor a
  ld [wPinPending], a
  ld a, [wPinReq]
  ld [wPin], a
  call SetLayout
  xor a
  ld [wLg], a
  ld [wSinceCut], a
  ret

; b = p, c = armed, a = from the wire gate. Only the latest request is kept.
SetRequest:
  ld [wReqWire], a
  ld a, b
  ld [wReqP], a
  ld a, c
  ld [wReqArmed], a
  ld a, 1
  ld [wReqPending], a
  ret

; a = effective mode, e = family of the pending request.
RequestMode:
  ld a, [wReqP]
  ld e, a
  srl e
  srl e
  and 3
  cp 3
  ret nz
  ld a, [wReqArmed]
  or a
  ld a, 3
  ret nz
  ld a, 2
  ret

; ---------------------------------------------------------------------------
; Displayed levels, region shades and the registers for the next VBlank.
BuildField:
  call Regions
  ld [wN], a
  ld a, [wMode]
  cp 1
  jr nz, .flat
  ld a, [wLvl]
  ld [wDisp], a
  ld a, [wLvl + 1]
  ld [wDisp + 1], a
  ld a, [wLvl + 2]
  ld [wDisp + 2], a
  jr .unused
.flat
  ld b, 3                ; held and cuts
  cp 3
  jr nz, .fill
  ld a, [wPulse]
  or a
  jr nz, .fill           ; lit
  ld a, [wDim]           ; rest: 1 when dim, else 0
  ld b, a
.fill
  ld a, b
  ld [wDisp], a
  ld [wDisp + 1], a
  ld [wDisp + 2], a
.unused
  ld a, [wN]
  cp 3
  jr z, .shades
  xor a
  ld [wDisp + 2], a
.shades
  ; combo row = DmgCombo + 16 * (n == 3) + 4 * combo
  ld a, [wN]
  sub 2
  swap a
  ld b, a
  ld a, [wCombo]
  add a
  add a
  add b
  ld b, a
  ld c, 0
.region
  ld a, c
  HLIDX wDisp
  ld d, [hl]
  ld a, [wMode]
  cp 1
  jr nz, .notSpectral
  ld a, 3                ; spectral: shade = 3 - L
  sub d
  jr .store
.notSpectral
  ld a, d
  cp 3
  jr nz, .rest
  ld a, [wN]             ; lit: combo shade of entry (i + rot) mod n
  ld d, a
  ld a, [wRot]
  add c
  cp d
  jr c, .entry
  sub d
.entry
  add b
  HLROM DmgCombo
  ld a, [hl]
  jr .store
.rest
  ld a, d                ; dim rest shade 2, dark rest shade 3
  or a
  ld a, 2
  jr nz, .store
  ld a, 3
.store
  ld e, a
  ld a, c
  HLIDX wShade
  ld [hl], e
  inc c
  ld a, [wN]
  cp c
  jr nz, .region
  ; BGP: colour k is class k, which shows region ClassRegion[layout][k].
  ld a, [wLayout]
  add a
  add a
  add 3
  HLROM ClassRegion
  ld b, 0
  ld c, 4
.bgp
  sla b
  sla b
  ld a, [hld]
  push hl
  HLIDX wShade
  ld a, [hl]
  or b
  ld b, a
  pop hl
  dec c
  jr nz, .bgp
  ld a, b
  ld [wShBGP], a
  ld a, [wLayout]
  and 1
  ld a, LCDC_FIELD_V
  jr z, .lcdc
  ld a, LCDC_FIELD_H
.lcdc
  ld [wShLCDC], a
  xor a
  ld [wShSCX], a
  ld [wShSCY], a
  ld a, SHOW_FIELD
  ld [wTraceShow], a
  ; The trace is the rendered state, taken before step 7's requests.
  ld hl, wMode
  ld de, wTrace
  ld c, 8
.trace
  ld a, [hli]
  ld [de], a
  inc de
  dec c
  jr nz, .trace
  ret

; ---------------------------------------------------------------------------
IF DEF(CHIPVIZ_INPUT_ROLE_OPERATOR)
; Operator role: a palette press on the entry step is already in wPEff; an
; unarmed 3 or 7 still opens the warning after the scene is initialised.
EntryPulse:
  ld a, [wOpPal]
  or a
  ret z
  xor a
  ld [wOpPal], a
  ld a, [wPalSel]
  and 3
  cp 3
  ret nz
  ld a, [wSceneIn]       ; leaving again this step (exempt): no warning
  cp 7
  ret nz
  ld a, [wPalSel]
  jp StartWarning

; Console warning screen (operator role only). The detector is paused.
StartWarning:
  ld [wWarnP], a
  xor a
  ld [wWarnLo], a
  ld [wWarnHi], a
  ld a, SHOW_WARN
  ld [wShowMode], a
  ret

ShowWarning:
  ld a, BGP_WARN
  ld [wShBGP], a
  ld a, LCDC_WARN
  ld [wShLCDC], a
  xor a
  ld [wShSCX], a
  ld a, SCY_WARN
  ld [wShSCY], a
  ld a, SHOW_WARN
  ld [wTraceShow], a
  ret

WarnStep:
  ld a, [wPadNew]
  or a
  jr z, .timer
  ld c, 0
  cp PAD_START
  jr nz, .leave          ; any other input cancels: unarmed, runs as mode 2
  ld a, [wShownShow]     ; Start before the warning reached the display:
  cp SHOW_WARN           ; ignored, it cannot confirm unseen text
  jr nz, .timer
  inc c                  ; Start: armed request, applied under the budget
  jr .leave
.timer
  ld hl, wWarnLo
  inc [hl]
  jr nz, .check
  inc hl
  inc [hl]
.check
  ld a, [wWarnLo]
  sub LOW(WARN_STEPS)
  ld a, [wWarnHi]
  sbc HIGH(WARN_STEPS)
  jp c, ShowWarning
  ld c, 0
.leave
  ld a, [wWarnP]
  ld b, a
  xor a
  call SetRequest
  ld a, SHOW_FIELD
  ld [wShowMode], a
  jp BuildField
ENDC

; ---------------------------------------------------------------------------
; VRAM for the colour field (LCD off).
LoadFieldVram:
  ; $8100: tiles 16..31 in $8000 mode, colour = column class (k & 3).
  ld hl, $8100
  ld b, 0
.colTiles
  ld a, b
  and 3
  call FillTile
  inc b
  ld a, b
  cp 16
  jr nz, .colTiles
  ; $9100: tiles 16..31 in $8800 mode, colour = row class (k >> 2).
  ld hl, $9100
  ld b, 0
.rowTiles
  ld a, b
  srl a
  srl a
  call FillTile
  inc b
  ld a, b
  cp 16
  jr nz, .rowTiles
  ; $9C00: rows 0..17, columns 0..19 = 16 + 4 * row class + column class.
  ld hl, $9C00
  ld d, 0                ; row
.mapRow
  ld e, 0                ; column
.mapCol
  ld a, d
  cp 18
  jr nc, .blank
  ld a, e
  cp 20
  jr nc, .blank
  push hl
  ld a, d
  HLROM RowClass
  ld a, [hl]
  add a
  add a
  ld c, a
  ld a, e
  HLROM ColClass
  ld a, [hl]
  add c
  add 16
  pop hl
  jr .put
.blank
  ld a, 16               ; colour 0 in both tile modes
.put
  ld [hli], a
  inc e
  ld a, e
  cp 32
  jr nz, .mapCol
  inc d
  ld a, d
  cp 32
  jr nz, .mapRow
IF DEF(CHIPVIZ_INPUT_ROLE_OPERATOR)
  ; Font at $8800 (tiles $80+), colour 3 glyphs.
  ld hl, $8800
  ld de, Font
  ld bc, FontEnd - Font
.font
  ld a, [de]
  inc de
  ld [hli], a
  ld [hli], a
  dec bc
  ld a, b
  or c
  jr nz, .font
  ; Seven lines on map rows 18, 20, ..., 30.
  ld hl, $9C00 + 18 * 32
  ld de, WarningText
  ld b, 7
.line
  ld c, 20
.char
  ld a, [de]
  inc de
  ld [hli], a
  dec c
  jr nz, .char
  push de
  ld de, 64 - 20
  add hl, de
  pop de
  dec b
  jr nz, .line
ENDC
  ret

; Writes 16 bytes of a solid tile of colour a (0..3) at hl. Keeps b.
FillTile:
  ld c, 0
  bit 0, a
  jr z, .hi
  ld c, $FF
.hi
  ld d, 0
  bit 1, a
  jr z, .write
  ld d, $FF
.write
  ld e, 8
.row
  ld a, c
  ld [hli], a
  ld a, d
  ld [hli], a
  dec e
  jr nz, .row
  ret

UpTab:
  db 0, 48, 104, 168
DownTab:
  db 0, 32, 88, 152

; Tile row (y / 8) and column (x / 8) classes.
RowClass:
  db 0, 0, 0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3
ColClass:
  db 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3

; Region shown by each class (colour 0..3) for V2, H2, V3 and H3. Rows are
; counted from the bottom, so row class 3 (the bottom rows) is region 0.
ClassRegion:
  db 0, 0, 1, 1
  db 1, 1, 0, 0
  db 0, 1, 1, 2
  db 2, 1, 1, 0

; DMG gating variant: shade (0 lightest .. 3 black) of combo entries, the
; same for both families. 2 regions, then 3 regions; 4 bytes per combo.
DmgCombo:
  db 0, 3, 0, 0,  1, 3, 0, 0,  0, 2, 0, 0,  3, 1, 0, 0
  db 0, 3, 1, 0,  3, 0, 2, 0,  1, 3, 0, 0,  0, 2, 3, 0

IF DEF(CHIPVIZ_INPUT_ROLE_OPERATOR)
; 5x7 glyphs, one byte per row, mapped to tiles $80+ by the charmap below.
MACRO GLYPH
  db (\1) << 2, (\2) << 2, (\3) << 2, (\4) << 2, (\5) << 2, (\6) << 2, (\7) << 2, 0
ENDM
Font:
  GLYPH $0E, $11, $11, $1F, $11, $11, $11   ; A
  GLYPH $1E, $11, $11, $1E, $11, $11, $1E   ; B
  GLYPH $0E, $11, $10, $10, $10, $11, $0E   ; C
  GLYPH $1F, $10, $10, $1E, $10, $10, $1F   ; E
  GLYPH $1F, $10, $10, $1E, $10, $10, $10   ; F
  GLYPH $0E, $11, $10, $17, $11, $11, $0F   ; G
  GLYPH $11, $11, $11, $1F, $11, $11, $11   ; H
  GLYPH $0E, $04, $04, $04, $04, $04, $0E   ; I
  GLYPH $10, $10, $10, $10, $10, $10, $1F   ; L
  GLYPH $11, $1B, $15, $15, $11, $11, $11   ; M
  GLYPH $11, $11, $19, $15, $13, $11, $11   ; N
  GLYPH $0E, $11, $11, $11, $11, $11, $0E   ; O
  GLYPH $1E, $11, $11, $1E, $10, $10, $10   ; P
  GLYPH $1E, $11, $11, $1E, $14, $12, $11   ; R
  GLYPH $0F, $10, $10, $0E, $01, $01, $1E   ; S
  GLYPH $1F, $04, $04, $04, $04, $04, $04   ; T
  GLYPH $11, $11, $11, $11, $11, $11, $0E   ; U
  GLYPH $11, $11, $11, $11, $11, $0A, $04   ; V
  GLYPH $11, $11, $11, $15, $15, $15, $0A   ; W
  GLYPH $11, $11, $0A, $04, $04, $04, $04   ; Y
  GLYPH $1F, $01, $02, $04, $08, $10, $1F   ; Z
  GLYPH $00, $00, $00, $00, $00, $0C, $0C   ; .
  GLYPH $00, $0C, $0C, $00, $0C, $0C, $00   ; :
  GLYPH $00, $00, $1F, $00, $1F, $00, $00   ; =
FontEnd:

NEWCHARMAP warning
CHARMAP "A", $80
CHARMAP "B", $81
CHARMAP "C", $82
CHARMAP "E", $83
CHARMAP "F", $84
CHARMAP "G", $85
CHARMAP "H", $86
CHARMAP "I", $87
CHARMAP "L", $88
CHARMAP "M", $89
CHARMAP "N", $8A
CHARMAP "O", $8B
CHARMAP "P", $8C
CHARMAP "R", $8D
CHARMAP "S", $8E
CHARMAP "T", $8F
CHARMAP "U", $90
CHARMAP "V", $91
CHARMAP "W", $92
CHARMAP "Y", $93
CHARMAP "Z", $94
CHARMAP ".", $95
CHARMAP ":", $96
CHARMAP "=", $97
CHARMAP " ", 16

WarningText:
  db "WARNING: FLASHING   "
  db "LIGHT. MAY TRIGGER  "
  db "SEIZURES IN PEOPLE  "
  db "WITH PHOTOSENSITIVE "
  db "EPILEPSY.           "
  db "START = ENABLE      "
  db "OTHER = CANCEL      "
SETCHARMAP main
ENDC
