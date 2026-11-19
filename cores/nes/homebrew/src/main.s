; chipviz NES: native scrolling demo (scene 0) and the colour field (scene 7,
; shared/specs/colour-field-v0.md). Everything after reset runs from NMI: the
; VBlank uploads come first, then the pad is read and one step runs, which
; prepares the buffers for the next VBlank.
;
; Build with -D CHIPVIZ_INPUT_ROLE_OPERATOR for the `operator` input role;
; the default is the `data` role.

.segment "HEADER"
  .byte "NES", $1a
  .byte 2
  .byte 1
  .byte $00, $00
  .byte $00, $00, $00, $00, $00, $00, $00, $00

; Pad bits, in the order ReadPad shifts them in.
BTN_A      = $80
BTN_B      = $40
BTN_SELECT = $20
BTN_START  = $10
BTN_UP     = $08
BTN_DOWN   = $04
BTN_LEFT   = $02
BTN_RIGHT  = $01

; Detector and rate constants from the spec.
SIL_AVG      = 12
SIL_EXIT     = 24
SIL_HOLD     = 30
T_FLOOR      = 24
T_E_ON       = 40
T_E_OFF      = 16
T_B_ON       = 56
T_B_OFF      = 24
FLUX_ON      = 200
FLUX_OFF     = 64
NOTE_VEL     = 64
DOM_MARGIN   = 32
DOM_HOLD     = 30
SECTION_MIN  = 600
CUT_MIN      = 20
LEVEL_OPP    = 10
GATE_ON      = 4
GATE_OFF_MIN = 4
REST_DIM_ON  = 64
REST_DIM_OFF = 48
P_STABLE     = 30
HID_E_LO     = 64
WARN_STEPS   = 600

SHOW_DEMO  = 0
SHOW_FIELD = 1
SHOW_WARN  = 2
UPLOAD_ATTR = $01
UPLOAD_PAL  = $02
PIN_AUTO    = $ff
BLACK       = $0f

; Saturating 8-bit increment. 8-bit counters are compared only against
; thresholds below 255, so they behave exactly like the spec's u16 counters.
.macro incsat var
  .local done
  inc var
  bne done
  dec var
done:
.endmacro

.segment "ZEROPAGE"
; Trace tuple (mode, family, layout, combo, rot, lvl0..2), kept contiguous.
Mode:         .res 1   ; effective mode 0..3 (3 only while the latch is set)
Family:       .res 1
Layout:       .res 1
Combo:        .res 1
Rot:          .res 1
Disp:         .res 3   ; displayed level per region
StepLo:       .res 1   ; step clock (one step per NMI)
StepHi:       .res 1
Frame:        .res 1
NmiBusy:      .res 1
Tmp:          .res 1
Tmp2:         .res 1
; Pad and source.
Pad:          .res 1
PadPrev:      .res 1
PadNew:       .res 1
PadSource:    .res 1   ; nonzero once B or A was held: detector reads the pad
SceneRing:    .res 1   ; incoming scene: 0 (native demo) or 7
PalRing:      .res 1   ; data role: incoming palette (relative pulses)
; Procedural generator (chipviz_frame_make_procedural, scene/palette unused).
ProcLo:       .res 1
ProcHi:       .res 1
ProcPhase:    .res 1   ; frame * 17
ProcMid:      .res 1   ; frame * 9
ProcTreble:   .res 1   ; frame * 23
ProcNote:     .res 1   ; (frame / 16) % 24
; Step input: InVals = e, bass, mid, treble.
InVals:       .res 4
InSpec:       .res 8
InFlags:      .res 1
InNoteN:      .res 1
InNote:       .res 1
InVel:        .res 1
PrevSpec:     .res 8
PrevFlags:    .res 1
PrevNoteN:    .res 1
PrevNote:     .res 1
PrevVel:      .res 1
AccLo:        .res 4
AccHi:        .res 4
DPos:         .res 4   ; max(0, x - avg_x) before the update
AvgB:         .res 3
FluxLo:       .res 1
FluxHi:       .res 1
; Detector and mode state.
TArmed:       .res 1
Silent:       .res 1
Quiet:        .res 1
Dom:          .res 1
Cand:         .res 1
CandN:        .res 1
EvT:          .res 1
EvN:          .res 1
EvB:          .res 1
EvX:          .res 1
EvE:          .res 1
EvD:          .res 1
Lvl:          .res 3
Act:          .res 1
Pulse:        .res 1
Hyst:         .res 1
Dim:          .res 1
SinceCut:     .res 1
SinceLayLo:   .res 1
SinceLayHi:   .res 1
GateOff:      .res 1
SincePulse:   .res 1
Lg:           .res 1
PEff:         .res 1
Latch:        .res 1   ; flash latch
Pin:          .res 1
PinReq:       .res 1
PinPending:   .res 1
ReqPending:   .res 1
ReqP:         .res 1
ReqArmed:     .res 1
; Scene-7 debounce.
Shown7:       .res 1
EntryPending: .res 1
ExitPending:  .res 1
DebVal:       .res 1
DebN:         .res 1
; Wire palette gate (data role).
WireVal:      .res 1
WireN:        .res 1
WireUsed:     .res 1
; Operator role.
PalSel:       .res 1
OpPal:        .res 1
OpPin:        .res 1
WarnP:        .res 1
WarnLo:       .res 1
WarnHi:       .res 1
; Output.
ShowMode:     .res 1
UploadFlags:  .res 1
AttrShown:    .res 1
AttrPtr:      .res 2
PalBuf:       .res 16
View:         .res 8   ; trace rendered by the last BuildField (Mode..Disp)

.segment "BSS"
; RAM starts on a page boundary for OAM DMA; the stack uses $0100-$01ff.
Oam:
  .res $100

.segment "CODE"
Reset:
  sei
  cld
  ldx #$40
  stx $4017
  ldx #$ff
  txs
  inx
  stx $2000
  stx $2001
  stx $4010
  txa
ClearZp:
  sta $00,x
  inx
  bne ClearZp
  lda #PIN_AUTO
  sta Pin
  sta PinReq
  sta Cand
  sta AttrShown
  lda #$ff
HideSprites:
  sta Oam,x
  inx
  bne HideSprites
  bit $2002
WaitVBlank1:
  bit $2002
  bpl WaitVBlank1
WaitVBlankEnd:
  bit $2002
  bmi WaitVBlankEnd
WaitVBlank2:
  bit $2002
  bpl WaitVBlank2
  jsr LoadPalette
  ; Horizontal mirroring: $2000/$2400 and $2800/$2c00 are aliases.
  lda #$20
  jsr LoadNameTable
  lda #$28
  jsr LoadNameTable
.ifdef CHIPVIZ_INPUT_ROLE_OPERATOR
  jsr LoadWarningText
.endif
  ; Finish setup in a fresh vblank. The status read also clears pending NMI
  ; and the address/scroll latch before enabling NMI and rendering.
  bit $2002
WaitStart:
  bit $2002
  bpl WaitStart
  jsr LoadOam
  lda #$00
  sta $2005
  sta $2005
  ; Restore nametable 0 after the palette/nametable address writes.
  lda #%10000000
  sta $2000
  lda #%00011110
  sta $2001
Forever:
  jmp Forever

Nmi:
  pha
  txa
  pha
  tya
  pha
  lda NmiBusy
  beq NmiStart
  jmp NmiExit            ; a late step drops this frame instead of tearing
NmiStart:
  inc NmiBusy
  inc Frame
  bit $2002
  ; VBlank uploads: at most 64 attribute and 16 palette bytes.
  lda UploadFlags
  and #UPLOAD_ATTR
  beq NmiPalette
  lda #$23
  sta $2006
  lda #$c0
  sta $2006
  ldy #$00
NmiAttrLoop:
  lda (AttrPtr),y
  sta $2007
  iny
  cpy #64
  bne NmiAttrLoop
NmiPalette:
  lda UploadFlags
  and #UPLOAD_PAL
  beq NmiPpu
  lda #$3f
  sta $2006
  lda #$00
  sta $2006
  ldx #$00
NmiPalLoop:
  lda PalBuf,x
  sta $2007
  inx
  cpx #16
  bne NmiPalLoop
NmiPpu:
  lda #$00
  sta UploadFlags
  lda ShowMode
  beq NmiDemo
  ; Colour field: nametable $2000, background tiles from $1000 (all solid),
  ; scroll 0,0, background only. Warning: nametable $2800 at Y=128, font.
  ldx #%10010000
  ldy #$00
  cmp #SHOW_WARN
  bne NmiFieldPpu
  ldx #%10000010
  ldy #128
NmiFieldPpu:
  stx $2000
  lda #$00
  sta $2005
  sty $2005
  lda #%00001010
  sta $2001
  jmp NmiLogic
NmiDemo:
  jsr LoadOam
  ; Keep $3f00 (the universal background) fixed. Only one foreground
  ; color changes, every 32 frames, without cycling brightness levels.
  lda Frame
  and #$1f
  bne NmiScroll
  lda #$3f
  sta $2006
  lda #$01
  sta $2006
  jsr DemoForeground
  sta $2007
NmiScroll:
  lda #%10000000
  sta $2000
  lda #$00
  sta $2005
  lda Frame
  lsr a
  sta $2005
  lda #%00011110
  sta $2001
NmiLogic:
  jsr ReadPad
  jsr Step
  dec NmiBusy
NmiExit:
  pla
  tay
  pla
  tax
  pla
  rti

Irq:
  rti

DemoForeground:
  lda Frame
  lsr a
  lsr a
  lsr a
  lsr a
  lsr a
  tax
  lda ForegroundColors,x
  rts

LoadOam:
  lda #$00
  sta $2003
  lda #>Oam
  sta $4014
  rts

ReadPad:
  lda Pad
  sta PadPrev
  lda #$01
  sta $4016
  lda #$00
  sta $4016
  ldx #8
ReadPadLoop:
  lda $4016
  lsr a
  rol Pad
  dex
  bne ReadPadLoop
  lda PadPrev
  eor #$ff
  and Pad
  sta PadNew
  rts

; ---------------------------------------------------------------------------
; One step per NMI.
Step:
  inc StepLo
  bne StepInput
  inc StepHi
StepInput:
  jsr MakeInput
.ifdef CHIPVIZ_INPUT_ROLE_OPERATOR
  lda ShowMode
  cmp #SHOW_WARN
  bne StepRings
  jmp WarnStep
StepRings:
.endif
  jsr UpdateRings
  ; A scene change decided at the end of step t shows from step t + 1.
  lda ExitPending
  beq StepScene
  lda #$00
  sta ExitPending
  jsr SceneExit
  jmp StepDebounce
StepScene:
  lda Shown7
  beq StepDebounce
  lda EntryPending
  beq StepField
  lda #$00
  sta EntryPending
  jsr SceneEntry
  jsr BuildField
.ifdef CHIPVIZ_INPUT_ROLE_OPERATOR
  jsr EntryPulse
.endif
  jmp StepDebounce
StepField:
  jsr FieldStep          ; renders step t, then applies requests for t + 1
StepDebounce:
  jmp SceneDebounce

; Step input. Procedural until B or A is first held, then the pad as a
; digital-only USB HID receiver: energy and bands are HID_E_LO (the NES pad
; has no shoulder), spectrum 0, B = beat, A = bar, no notes.
MakeInput:
  lda PadSource
  bne PadInput
  lda Pad
  and #(BTN_A | BTN_B)
  beq ProcInput
  sta PadSource
PadInput:
  lda #HID_E_LO
  ldx #3
PadVals:
  sta InVals,x
  dex
  bpl PadVals
.ifdef CHIPVIZ_TEST_SILENCE
  lda Pad
  and #BTN_SELECT        ; test build only: Select zeroes the energy (bands stay)
  beq PadLoud
  lda #$00
  sta InVals
PadLoud:
.endif
  lda #$00
  sta InNoteN
  ldx #7
PadSpec:
  sta InSpec,x
  dex
  bpl PadSpec
  ldx #$00
  lda Pad
  and #BTN_B
  beq PadBar
  ldx #$01
PadBar:
  lda Pad
  and #BTN_A
  beq PadFlags
  inx
  inx
PadFlags:
  stx InFlags
  rts
ProcInput:
  lda ProcPhase
  lsr a
  clc
  adc #80
  sta InVals
  lda ProcPhase
  eor #$ff
  sta InVals+1
  lda ProcMid
  and #$7f
  clc
  adc #64
  sta InVals+2
  lda ProcTreble
  sta InVals+3
  lda ProcPhase
  ldx #$00
ProcSpec:
  sta InSpec,x
  clc
  adc #29
  inx
  cpx #8
  bne ProcSpec
  lda #$00
  sta InFlags
  sta InNoteN
  lda ProcPhase
  cmp #17
  bcs ProcAdvance
  inc InFlags
  inc InNoteN
  lda ProcNote
  clc
  adc #48
  sta InNote
  lda InVals
  sta InVel
ProcAdvance:
  lda ProcPhase
  clc
  adc #17
  sta ProcPhase
  lda ProcMid
  clc
  adc #9
  sta ProcMid
  lda ProcTreble
  clc
  adc #23
  sta ProcTreble
  inc ProcLo
  bne ProcBeat
  inc ProcHi
  bne ProcBeat
  lda #$00               ; frame index wrapped to 0
  sta ProcNote
  rts
ProcBeat:
  lda ProcLo
  and #$0f
  bne ProcDone
  inc ProcNote
  lda ProcNote
  cmp #24
  bcc ProcDone
  lda #$00
  sta ProcNote
ProcDone:
  rts

; Scene ring (0, 7) on Up/Down and palette ring on Right/Left. Pressed edges
; only; opposing held directions cancel on their axis.
UpdateRings:
  lda Pad
  and #(BTN_UP | BTN_DOWN)
  cmp #(BTN_UP | BTN_DOWN)
  beq RingPalette
  lda PadNew
  and #(BTN_UP | BTN_DOWN)
  beq RingPalette
  lda SceneRing
  eor #7
  sta SceneRing
RingPalette:
  lda Pad
  and #(BTN_LEFT | BTN_RIGHT)
  cmp #(BTN_LEFT | BTN_RIGHT)
  beq RingPin
  lda PadNew
  and #BTN_RIGHT
  beq RingLeft
  lda #1
  bne RingApply
RingLeft:
  lda PadNew
  and #BTN_LEFT
  beq RingPin
  lda #$ff
RingApply:
.ifdef CHIPVIZ_INPUT_ROLE_OPERATOR
  clc
  adc PalSel
  and #7
  sta PalSel
  lda Shown7             ; outside scene 7 nothing is shown: no pulse
  beq RingPin
  inc OpPal
.else
  clc
  adc PalRing
  and #7
  sta PalRing
.endif
RingPin:
.ifdef CHIPVIZ_INPUT_ROLE_OPERATOR
  lda PadNew
  and #BTN_SELECT
  beq RingDone
  ; auto -> V2 -> H2 -> V3 -> H3 -> auto
  lda PinReq
  clc
  adc #1
  cmp #4
  bne RingPinSet
  lda #PIN_AUTO
RingPinSet:
  sta PinReq
  inc OpPin
  lda Shown7
  beq RingPinNow
  lda EntryPending       ; entering this step: the entry uses Pin
  beq RingDone
RingPinNow:
  lda PinReq             ; not shown: nothing visible, apply now
  sta Pin
  lda #$00
  sta OpPin
RingDone:
.endif
  rts

; Scene-7 debounce at the end of the step. Data role: a change needs the
; incoming scene unchanged for P_STABLE steps. Operator role: local Up/Down
; scene inputs are exempt and take effect at once.
SceneDebounce:
  lda SceneRing
  cmp DebVal
  beq DebSame
  sta DebVal
  lda #$00
  sta DebN
DebSame:
  incsat DebN
  ldx #$00
  lda SceneRing
  cmp #7
  bne DebWant
  inx
DebWant:
  cpx Shown7
  beq DebDone
.ifndef CHIPVIZ_INPUT_ROLE_OPERATOR
  lda DebN
  cmp #P_STABLE
  bcc DebDone
.endif
  stx Shown7
  txa
  bne DebEnter
  lda #1
  sta ExitPending
  rts
DebEnter:
  sta EntryPending
DebDone:
  rts
SceneExit:
  lda #SHOW_DEMO
  sta ShowMode
  sta Latch
  sta OpPal
  sta OpPin
  lda #PIN_AUTO
  sta AttrShown
  lda #<DemoAttr
  sta AttrPtr
  lda #>DemoAttr
  sta AttrPtr+1
  ldx #15
ExitPal:
  lda Palettes,x
  sta PalBuf,x
  dex
  bpl ExitPal
  jsr DemoForeground
  sta PalBuf+1
  lda #(UPLOAD_ATTR | UPLOAD_PAL)
  sta UploadFlags
  rts

; ---------------------------------------------------------------------------
; Entry step: initialise, render, no events. Never the power-on entry on the
; NES (the scene ring powers on at 0), so it always uses the change budget.
SceneEntry:
  ldx #3
EntryAcc:
  lda InVals,x
  sta AccLo,x
  lda #$00
  sta AccHi,x
  asl AccLo,x
  rol AccHi,x
  asl AccLo,x
  rol AccHi,x
  asl AccLo,x
  rol AccHi,x
  dex
  bpl EntryAcc
  ldx #7
EntrySpec:
  lda InSpec,x
  sta PrevSpec,x
  dex
  bpl EntrySpec
  jsr SavePrevInput
  lda #1
  sta TArmed
  lda #$00
  sta Silent
  sta Quiet
  sta CandN
  sta Rot
  sta Combo
  sta Latch
  sta ReqPending
  sta PinPending
  sta OpPin
  sta SinceLayLo
  sta SinceLayHi
  lda #PIN_AUTO
  sta Cand
  ; dom = argmax of the raw bands, ties to the lowest index
  ldx #$00
  lda InVals+2
  cmp InVals+1
  bcc EntryDom2
  beq EntryDom2
  inx
EntryDom2:
  lda InVals+3
  cmp InVals+1,x
  bcc EntryDomSet
  beq EntryDomSet
  ldx #2
EntryDomSet:
  stx Dom
  lda Pin
  sta PinReq
  cmp #PIN_AUTO
  bne EntryLayout
  lda #$00
EntryLayout:
  sta Layout
.ifdef CHIPVIZ_INPUT_ROLE_OPERATOR
  lda PalSel
.else
  lda PalRing
  sta WireVal
  ldx #1
  stx WireN
  ldx #$00
  stx WireUsed
.endif
  sta PEff
  jsr SetModeFromPEff    ; unarmed: a 3 or 7 runs as 2 or 6
  lda #(GATE_ON + GATE_OFF_MIN)
  sta SincePulse
  jsr ModeEntry
  lda #SHOW_FIELD
  sta ShowMode
  rts

; Mode and family from PEff and the latch (3 without the latch runs as 2).
SetModeFromPEff:
  lda PEff
  lsr a
  lsr a
  sta Family
  lda PEff
  and #3
  cmp #3
  bne SetMode
  ldx Latch
  bne SetMode
  lda #2
SetMode:
  sta Mode
  rts

; Mode entry: on scene entry and on every mode or family change.
ModeEntry:
  lda #$00
  sta Act
  sta SinceCut
  sta Lg
  sta Pulse
  lda #GATE_OFF_MIN
  sta GateOff
  lda Mode
  bne ModeEntryLevels
  lda Dom
  sta Combo
ModeEntryLevels:
  lda Mode
  cmp #1
  bne ModeEntryGate
  jsr InitLevels
ModeEntryGate:
  ldx #0
  jsr GetAvg
  ldx #$00
  cmp #REST_DIM_ON
  bcc ModeEntryDim
  inx
ModeEntryDim:
  stx Hyst
  stx Dim
  lda Silent
  beq ModeEntryDone
  lda Mode
  and #1                 ; spectral (1) or flash/gate (3)
  beq ModeEntryDone
  lda #$00
  sta Lvl
  sta Lvl+1
  sta Lvl+2
  sta Hyst
  sta Dim
ModeEntryDone:
  rts

SavePrevInput:
  lda InFlags
  sta PrevFlags
  lda InNoteN
  sta PrevNoteN
  lda InNote
  sta PrevNote
  lda InVel
  sta PrevVel
  rts

; A = avg of InVals[X] (acc >> 3).
GetAvg:
  lda AccHi,x
  sta Tmp
  lda AccLo,x
  lsr Tmp
  ror a
  lsr Tmp
  ror a
  lsr Tmp
  ror a
  rts

; A = region count (2 or 3) for Layout.
Regions:
  lda Layout
  lsr a
  clc
  adc #2
  rts

; A = v_i for region X: bass, mid, treble (3 regions) or bass,
; max(mid, treble) (2 regions).
RegionValue:
  lda Layout
  and #2
  bne RegionRaw
  cpx #1
  bne RegionRaw
  lda InVals+2
  cmp InVals+3
  bcs RegionMax
  lda InVals+3
RegionMax:
  rts
RegionRaw:
  lda InVals+1,x
  rts

; lvl[i] = count(UP[k] <= v_i); lvl[2] = 0 in 2-region layouts.
InitLevels:
  lda #$00
  sta Lvl+2
  jsr Regions
  sta Tmp2
  ldx #$00
InitLevelLoop:
  jsr RegionValue
  ldy #$00
  cmp UpTab+1
  bcc InitLevelSet
  iny
  cmp UpTab+2
  bcc InitLevelSet
  iny
  cmp UpTab+3
  bcc InitLevelSet
  iny
InitLevelSet:
  sty Lvl,x
  inx
  cpx Tmp2
  bne InitLevelLoop
  rts

; Layout operations.
NextLayout:
  lda Pin
  cmp #PIN_AUTO
  bne NextLayoutDone     ; pinned: event-driven next_layout does nothing
  lda Layout
  clc
  adc #1
  and #3
SetLayout:
  sta Layout
  jsr Regions
  cmp Rot
  beq SetLayoutRot
  bcs SetLayoutKept
SetLayoutRot:
  lda #$00               ; rot = rot mod n (rot <= 2)
  sta Rot
SetLayoutKept:
  lda #$00
  sta SinceLayLo
  sta SinceLayHi
  sta Lg
  lda Mode
  cmp #1
  bne NextLayoutDone
  jmp InitLevels
NextLayoutDone:
  rts

NextCombo:
  lda Combo
  clc
  adc #1
  and #3
  sta Combo
  rts

Rotate:
  jsr Regions
  sta Tmp2
  ldx Rot
  inx
  cpx Tmp2
  bcc RotateSet
  ldx #$00
RotateSet:
  stx Rot
  rts

; ---------------------------------------------------------------------------
; Steps 1-7 of every later step.
FieldStep:
  ; 1. Deltas against the previous averages, then the EMA update.
  ldx #3
FieldEma:
  jsr GetAvg
  sta Tmp2
  lda InVals,x
  sec
  sbc Tmp2
  bcs FieldDelta
  lda #$00
FieldDelta:
  sta DPos,x
  lda AccLo,x
  sec
  sbc Tmp2
  sta AccLo,x
  bcs FieldAccAdd
  dec AccHi,x
FieldAccAdd:
  lda AccLo,x
  clc
  adc InVals,x
  sta AccLo,x
  bcc FieldEmaNext
  inc AccHi,x
FieldEmaNext:
  dex
  bpl FieldEma
  lda #$00
  sta FluxLo
  sta FluxHi
  ldx #7
FieldFlux:
  lda InSpec,x
  sec
  sbc PrevSpec,x
  bcc FieldFluxNext
  clc
  adc FluxLo
  sta FluxLo
  bcc FieldFluxNext
  inc FluxHi
FieldFluxNext:
  lda InSpec,x
  sta PrevSpec,x
  dex
  bpl FieldFlux

  ; 2. Silence.
  lda Silent
  beq SilQuiet
  lda InVals
  cmp #SIL_EXIT
  bcc SilBlackout
  lda #$00
  sta Silent
  sta Quiet
  beq SilBlackout
SilQuiet:
  ldx #0
  jsr GetAvg
  cmp #(SIL_AVG + 1)
  bcs SilLoud
  incsat Quiet
  jmp SilCheck
SilLoud:
  lda #$00
  sta Quiet
SilCheck:
  lda Quiet
  cmp #SIL_HOLD
  bcc SilBlackout
  lda #1
  sta Silent
  lda #$00
  sta Dim
  lda Pulse
  beq SilBlackout
  lda #$00
  sta Pulse
  sta GateOff
SilBlackout:
  lda Mode
  cmp #1
  bne Transients
  lda Silent
  beq Transients
  lda Lvl
  ora Lvl+1
  ora Lvl+2
  beq Transients
  lda Lg
  cmp #LEVEL_OPP
  bcc Transients
  lda #$00
  sta Lvl
  sta Lvl+1
  sta Lvl+2
  sta Lg

  ; 3. Transients. hit and calm are mutually exclusive.
Transients:
  lda #$00
  sta EvT
  lda InVals
  cmp #T_FLOOR
  bcc CalmCheck
  lda DPos
  cmp #T_E_ON
  bcs IsHit
  ldx #3
HitBands:
  lda DPos,x
  cmp #T_B_ON
  bcs IsHit
  dex
  bne HitBands
  lda FluxHi
  bne IsHit
  lda FluxLo
  cmp #FLUX_ON
  bcc CalmCheck
IsHit:
  lda TArmed
  beq Events
  lda #$00
  sta TArmed
  inc EvT
  bne Events
CalmCheck:
  lda DPos
  cmp #T_E_OFF
  bcs Events
  ldx #3
CalmBands:
  lda DPos,x
  cmp #T_B_OFF
  bcs Events
  dex
  bne CalmBands
  lda FluxHi
  bne Events
  lda FluxLo
  cmp #FLUX_OFF
  bcs Events
  lda #1
  sta TArmed

  ; 4. Events.
Events:
  lda #$00
  sta EvN
  lda InNoteN
  beq EventFlags
  lda InVel
  cmp #NOTE_VEL
  bcc EventFlags
  lda PrevNoteN
  beq EventNote
  lda InNote
  cmp PrevNote
  bne EventNote
  lda InVel
  cmp PrevVel
  beq EventFlags
EventNote:
  inc EvN
EventFlags:
  lda PrevFlags
  eor #$ff
  and InFlags
  tax
  and #$03
  sta EvB
  txa
  and #$04
  sta EvX
  jsr SavePrevInput
  lda #$00
  sta EvE
  lda Silent
  bne Dominance
  lda EvT
  ora EvN
  ora EvB
  ora EvX
  sta EvE

  ; 5. Dominance on the new averages.
Dominance:
  ldx #1
  jsr GetAvg
  sta AvgB
  ldx #2
  jsr GetAvg
  sta AvgB+1
  ldx #3
  jsr GetAvg
  sta AvgB+2
  ldx #$00
  lda AvgB+1
  cmp AvgB
  bcc DomC2
  beq DomC2
  inx
DomC2:
  lda AvgB+2
  cmp AvgB,x
  bcc DomC
  beq DomC
  ldx #2
DomC:
  lda #$00
  sta EvD
  lda Silent
  bne DomReset
  cpx Dom
  beq DomReset
  ldy Dom
  lda AvgB,y
  clc
  adc #DOM_MARGIN
  bcs DomReset
  sta Tmp2
  lda AvgB,x
  cmp Tmp2
  bcc DomReset
  cpx Cand
  beq DomInc
  stx Cand
  lda #1
  sta CandN
  bne DomCheck
DomInc:
  incsat CandN
DomCheck:
  lda CandN
  cmp #DOM_HOLD
  bcc ModeRule
  stx Dom
  lda #$00
  sta CandN
  inc EvD
  bne ModeRule
DomReset:
  lda #PIN_AUTO
  sta Cand
  lda #$00
  sta CandN

  ; 6. Mode rule, unless silent.
ModeRule:
  lda Silent
  bne Counters
  lda Mode
  bne ModeRule1
  jsr HeldRule
  jmp Counters
ModeRule1:
  cmp #1
  bne ModeRule2
  jsr SpectralRule
  jmp Counters
ModeRule2:
  cmp #2
  bne ModeRule3
  jsr CutsRule
  jmp Counters
ModeRule3:
  jsr GateRule

  ; 7. Counters, then requests under the change budget.
Counters:
  incsat SinceCut
  incsat GateOff
  incsat SincePulse
  incsat Lg
  inc SinceLayLo
  bne Requests
  inc SinceLayHi
  bne Requests
  lda #$ff
  sta SinceLayLo
  sta SinceLayHi
Requests:
  ; The output of step t is the state before its requests: a request applied
  ; here is visible from step t + 1.
  jsr BuildField
  jmp UpdateRequests

; SECTION = X || (D && since_layout >= SECTION_MIN), as carry.
Section:
  lda EvX
  bne SectionYes
  lda EvD
  beq SectionNo
  lda SinceLayLo
  cmp #<SECTION_MIN
  lda SinceLayHi
  sbc #>SECTION_MIN
  rts
SectionYes:
  sec
  rts
SectionNo:
  clc
  rts

HeldRule:
  lda EvD
  beq HeldSection
  lda Dom
  sta Combo
  lda #$00
  sta Lg
HeldSection:
  jsr Section
  bcc HeldDone
  lda SinceCut
  cmp #CUT_MIN
  bcc HeldDone
  jsr NextLayout
  lda #$00
  sta SinceCut
HeldDone:
  rts

SpectralRule:
  lda Lg
  cmp #LEVEL_OPP
  bcc SpecDone           ; X and D are dropped, and levels wait
  lda EvX
  beq SpecDom
  lda SinceCut
  cmp #CUT_MIN
  bcc SpecDom
  jsr NextLayout
  jsr NextCombo
  lda #$00
  sta SinceCut
  sta Lg
  rts
SpecDom:
  lda EvD
  beq SpecLevels
  lda SinceLayLo
  cmp #<SECTION_MIN
  lda SinceLayHi
  sbc #>SECTION_MIN
  bcc SpecLevels
  lda Pin
  cmp #PIN_AUTO
  bne SpecLevels
  jmp NextLayout         ; sets lg = 0, which blocks the level update
SpecLevels:
  jsr Regions
  sta Tmp2
  ldx #$00
  stx Tmp
SpecLevelLoop:
  jsr RegionValue
  ldy Lvl,x
  cpy #3
  bcs SpecDown
  cmp UpTab+1,y
  bcc SpecDown
  inc Lvl,x
  inc Tmp
  bne SpecNext
SpecDown:
  cpy #0
  beq SpecNext
  cmp DownTab,y
  bcs SpecNext
  dec Lvl,x
  inc Tmp
SpecNext:
  inx
  cpx Tmp2
  bne SpecLevelLoop
  lda Tmp
  beq SpecDone
  lda #$00
  sta Lg
SpecDone:
  rts

CutsRule:
  lda EvE
  beq CutsDone
  lda SinceCut
  cmp #CUT_MIN
  bcc CutsDone
  lda #$00
  sta SinceCut
  sta Lg
  lda EvX
  beq CutsAct
  jsr NextLayout
  jmp NextCombo
CutsAct:
  lda Act
  tax
  clc
  adc #1
  and #3
  sta Act
  txa
  and #1
  beq CutsRotate         ; actions 0 and 2
  cpx #1
  bne CutsLayout
  jmp NextCombo
CutsLayout:
  lda Pin
  cmp #PIN_AUTO
  bne CutsRotate         ; pinned: action 3 rotates instead
  jmp NextLayout
CutsRotate:
  jmp Rotate
CutsDone:
  rts

GateRule:
  ldx #0
  jsr GetAvg
  cmp #REST_DIM_ON
  bcc GateHystOff
  ldx #1
  stx Hyst
  bne GatePulse
GateHystOff:
  cmp #REST_DIM_OFF
  bcs GatePulse
  ldx #$00
  stx Hyst
GatePulse:
  lda Pulse
  beq GateStart
  dec Pulse
  bne GateDone
  lda #$00
  sta GateOff
  lda Hyst
  sta Dim
  rts
GateStart:
  lda EvE
  beq GateDone
  lda GateOff
  cmp #GATE_OFF_MIN
  bcc GateDone
  lda SincePulse
  cmp #(GATE_ON + GATE_OFF_MIN)
  bcc GateDone
  lda #GATE_ON
  sta Pulse
  lda #$00
  sta SincePulse
  jsr Rotate
  lda EvX
  beq GateDone
  jmp NextCombo
GateDone:
  rts

; Operator role: a palette press on the entry step is already in PEff; an
; unarmed 3 or 7 still opens the warning after the scene is initialised.
.ifdef CHIPVIZ_INPUT_ROLE_OPERATOR
EntryPulse:
  lda OpPal
  beq EntryPulseDone
  lda #$00
  sta OpPal
  lda PalSel
  and #3
  cmp #3
  bne EntryPulseDone
  lda SceneRing          ; leaving again this step (exempt): no warning
  cmp #7
  bne EntryPulseDone
  lda PalSel
  jmp StartWarning
EntryPulseDone:
  rts
.endif

; Step 7 requests: palette gate or operator pulses, pin, then the budget.
UpdateRequests:
.ifdef CHIPVIZ_INPUT_ROLE_OPERATOR
  lda OpPal
  beq ReqPinPulse
  lda #$00
  sta OpPal
  lda PalSel
  and #3
  cmp #3
  bne ReqOperator
  lda Latch
  bne ReqOperator
  lda SceneRing          ; leaving scene 7 this step (exempt): no warning
  cmp #7
  bne ReqOperator
  lda PalSel             ; unarmed mode 3: show the warning screen
  jmp StartWarning
ReqOperator:
  lda PalSel
  ldx Latch              ; already armed: a 3 or 7 stays armed
  jsr SetRequest
ReqPinPulse:
  lda OpPin
  beq ReqClearEqual
  lda #$00
  sta OpPin
  lda #1
  sta PinPending
.else
  ; Wire gate. The ring is relative, so a stable value is never armed. A
  ; change is tracked in every step, so a new value is never left consumed.
  lda PalRing
  cmp WireVal
  beq WireSame
  sta WireVal
  lda #$00
  sta WireN
  sta WireUsed
WireSame:
  lda SceneRing
  cmp #7
  beq WireCount
  lda #$00               ; outside the scene-7 window
  sta WireN
  sta ReqPending
  beq ReqClearEqual
WireCount:
  incsat WireN
  lda WireN
  cmp #P_STABLE
  bne ReqClearEqual
  lda WireUsed
  bne ReqClearEqual
  inc WireUsed
  lda WireVal
  ldx #$00
  jsr SetRequest
.endif
ReqClearEqual:
  ; A request equal in effective mode and family is cleared without effect.
  lda ReqPending
  beq ReqPinFree
  jsr RequestMode
  cmp Mode
  bne ReqPinFree
  cpx Family
  bne ReqPinFree
  lda #$00
  sta ReqPending
ReqPinFree:
  ; Setting auto, or pinning the shown layout, needs no budget.
  lda PinPending
  beq ReqBudget
  lda PinReq
  cmp #PIN_AUTO
  beq ReqPinApplyFree
  cmp Layout
  bne ReqBudget
ReqPinApplyFree:
  sta Pin
  lda #$00
  sta PinPending
ReqBudget:
  lda Lg
  cmp #LEVEL_OPP
  bcc ReqDone
  lda SinceCut
  cmp #CUT_MIN
  bcc ReqDone
  lda ReqPending
  beq ReqPin
  lda #$00
  sta ReqPending
  lda ReqP
  sta PEff
  jsr RequestMode
  sta Mode
  stx Family
  ldx #$00
  cmp #3
  bne ReqLatch
  inx                    ; mode 3 here means the request was armed
ReqLatch:
  stx Latch
  jmp ModeEntry
ReqPin:
  lda PinPending
  beq ReqDone
  lda #$00
  sta PinPending
  lda PinReq
  sta Pin
  jsr SetLayout
  lda #$00
  sta Lg
  sta SinceCut
ReqDone:
  rts

; A = p, X = armed. Only the latest request is kept.
SetRequest:
  sta ReqP
  stx ReqArmed
  lda #1
  sta ReqPending
  rts

; A = effective mode, X = family of the pending request.
RequestMode:
  lda ReqP
  lsr a
  lsr a
  tax
  lda ReqP
  and #3
  cmp #3
  bne RequestModeDone
  ldy ReqArmed
  bne RequestModeDone
  lda #2
RequestModeDone:
  rts

.ifdef CHIPVIZ_INPUT_ROLE_OPERATOR
; Console warning screen (operator role only). The detector is paused.
StartWarning:
  sta WarnP
  lda #PIN_AUTO          ; drops any queued attribute upload: redo it on return
  sta AttrShown
  lda #$00
  sta WarnLo
  sta WarnHi
  lda #SHOW_WARN
  sta ShowMode
  ldx #15
  lda #BLACK
WarnPalClear:
  sta PalBuf,x
  dex
  bpl WarnPalClear
  lda #$30               ; text: palette 3, colour 1, white
  sta PalBuf+13
  lda #UPLOAD_PAL
  sta UploadFlags
  rts

WarnStep:
  lda PadNew
  beq WarnTimer
  ldx #$00
  cmp #BTN_START
  bne WarnLeave          ; any other input cancels: unarmed, runs as mode 2
  inx                    ; START: armed request, applied under the budget
  bne WarnLeave
WarnTimer:
  inc WarnLo
  bne WarnTimerCheck
  inc WarnHi
WarnTimerCheck:
  lda WarnLo
  cmp #<WARN_STEPS
  lda WarnHi
  sbc #>WARN_STEPS
  bcc WarnDone
  ldx #$00
WarnLeave:
  lda WarnP
  jsr SetRequest
  lda #SHOW_FIELD
  sta ShowMode
  jmp BuildField
WarnDone:
  rts

; Rows 16-29 of nametable $2800 hold the warning text. The demo scrolls at
; most 127 lines, so it never shows them. Their attributes select palette 3.
LoadWarningText:
  bit $2002
  lda #$2a
  sta $2006
  lda #$00
  sta $2006
  ldy #$00               ; text offset
  ldx #$00               ; row 0..13
WarnRow:
  lda WarnRows,x
  beq WarnBlankRow
  lda #$20
  jsr WarnSpaces6
  lda #20
  sta Tmp
WarnChar:
  lda WarningText,y
  sta $2007
  iny
  dec Tmp
  bne WarnChar
  lda #$20
  jsr WarnSpaces6
  jmp WarnRowNext
WarnBlankRow:
  lda #32
  sta Tmp
  lda #$20
WarnBlankLoop:
  sta $2007
  dec Tmp
  bne WarnBlankLoop
WarnRowNext:
  inx
  cpx #14
  bne WarnRow
  lda #$2b
  sta $2006
  lda #$e0
  sta $2006
  lda #$ff
  ldx #32
WarnAttr:
  sta $2007
  dex
  bne WarnAttr
  rts

WarnSpaces6:
  sta $2007
  sta $2007
  sta $2007
  sta $2007
  sta $2007
  sta $2007
  rts

; Text rows start 16 px down, inside the NTSC safe area.
WarnRows:
  .byte 0, 0, 1, 1, 1, 1, 1, 0, 1, 0, 1, 0, 0, 0
WarningText:
  .byte "WARNING: FLASHING   "
  .byte "LIGHT. MAY TRIGGER  "
  .byte "SEIZURES IN PEOPLE  "
  .byte "WITH PHOTOSENSITIVE "
  .byte "EPILEPSY.           "
  .byte "START = ENABLE      "
  .byte "OTHER = CANCEL      "
.endif

; ---------------------------------------------------------------------------
; Displayed levels and the region palettes for the next VBlank.
BuildField:
  jsr Regions
  sta Tmp2
  lda Mode
  cmp #1
  bne BuildFlat
  ldx #2
BuildCopy:
  lda Lvl,x
  sta Disp,x
  dex
  bpl BuildCopy
  bmi BuildUnused
BuildFlat:
  ldx #3                 ; held and cuts
  cmp #3
  bne BuildFill
  lda Pulse
  bne BuildFill          ; lit
  ldx Dim                ; rest: 1 when dim, else 0
BuildFill:
  stx Disp
  stx Disp+1
  stx Disp+2
BuildUnused:
  lda Tmp2
  cmp #3
  beq BuildPalette
  lda #$00
  sta Disp+2
BuildPalette:
  ldx #15
  lda #BLACK
BuildClear:
  sta PalBuf,x
  dex
  bpl BuildClear
  ; combo row = ((family * 2 + is3) * 4 + combo) * 3
  lda Layout
  lsr a
  sta Tmp
  lda Family
  asl a
  ora Tmp
  asl a
  asl a
  ora Combo
  sta Tmp
  asl a
  clc
  adc Tmp
  sta Tmp
  ldx #$00               ; region i
BuildRegion:
  txa
  clc
  adc Rot
  cmp Tmp2
  bcc BuildEntry
  sbc Tmp2               ; (i + rot) mod n, carry is set
BuildEntry:
  clc
  adc Tmp
  tay
  lda ComboTab,y
  tay                    ; colour id
  lda Disp,x
  cmp #3
  bne BuildL2
  lda BaseColors,y
  jmp BuildStore
BuildL2:
  cmp #2
  bne BuildBlack
  lda L2Colors,y
  jmp BuildStore
BuildBlack:
  lda #BLACK             ; levels 1 and 0; never $0d
BuildStore:
  pha
  txa
  asl a
  asl a
  tay
  pla
  sta PalBuf+1,y         ; region i uses background palette i
  inx
  cpx Tmp2
  bne BuildRegion
  lda #UPLOAD_PAL
  ldx Layout
  cpx AttrShown
  beq BuildUpload
  stx AttrShown
  lda AttrLo,x
  sta AttrPtr
  lda AttrHi,x
  sta AttrPtr+1
  lda #(UPLOAD_ATTR | UPLOAD_PAL)
BuildUpload:
  sta UploadFlags
  ldx #7
BuildView:
  lda Mode,x
  sta View,x
  dex
  bpl BuildView
  rts

; ---------------------------------------------------------------------------
LoadPalette:
  bit $2002
  lda #$3f
  sta $2006
  lda #$00
  sta $2006
  ldx #$00
PaletteLoop:
  lda Palettes,x
  sta $2007
  inx
  cpx #$20
  bne PaletteLoop
  rts

LoadNameTable:
  ; A selects the nametable. Fill 960 tiles followed by 64 attributes.
  bit $2002
  sta $2006
  lda #$00
  sta $2006
  ldx #$00
  ldy #$03
NameLoopA:
  txa
  and #$03
  sta $2007
  inx
  bne NameLoopA
  dey
  bne NameLoopA
NameLoopB:
  txa
  lsr a
  and #$03
  sta $2007
  inx
  cpx #$c0
  bne NameLoopB
  ldx #$40
AttrLoop:
  lda #$e4
  sta $2007
  dex
  bne AttrLoop
  rts

ForegroundColors:
  .byte $11,$12,$13,$14,$15,$14,$13,$12

Palettes:
  .byte $0f,$11,$21,$31, $0f,$05,$15,$25, $0f,$09,$19,$29, $0f,$0c,$1c,$2c
  .byte $0f,$10,$20,$30, $0f,$06,$16,$26, $0f,$0a,$1a,$2a, $0f,$0f,$12,$30

UpTab:
  .byte 0, 48, 104, 168
DownTab:
  .byte 0, 32, 88, 152

; Colour ids: 0 red, 1 yellow, 2 blue, 3 white, 4 black, 5 green, 6 cyan,
; 7 magenta. Level 3 uses BaseColors and level 2 L2Colors (spec table).
BaseColors:
  .byte $16, $28, $12, $30, $0f, $19, $3c, $14
L2Colors:
  .byte $06, $18, $02, $10, $0f, $09, $2c, $04

; Combo entries for regions 0..2: [family][3 regions][combo].
C_R = 0
C_Y = 1
C_B = 2
C_W = 3
C_K = 4
C_G = 5
C_C = 6
C_M = 7
ComboTab:
  .byte C_R,C_B,0,   C_Y,C_B,0,   C_R,C_Y,0,   C_B,C_W,0     ; A, 2 regions
  .byte C_R,C_Y,C_B, C_R,C_W,C_B, C_Y,C_K,C_B, C_R,C_K,C_Y   ; A, 3 regions
  .byte C_R,C_C,0,   C_G,C_M,0,   C_B,C_Y,0,   C_C,C_M,0     ; B, 2 regions
  .byte C_R,C_G,C_B, C_C,C_M,C_Y, C_R,C_K,C_C, C_B,C_K,C_Y   ; B, 3 regions

; Attribute tables. Region i uses background palette i. Boundaries follow
; b_i = 16 * ((i * (S / 16)) / n): V2 128; V3 80, 160; rows from the bottom
; H2 112 (screen row 128); H3 80, 160 (screen rows 160, 80).
.macro region_of lay, bx, by
  .if lay = 0
    rv .set (bx >= 8)
  .elseif lay = 1
    rv .set 1 - (by >= 8)
  .elseif lay = 2
    rv .set (bx >= 5) + (bx >= 10)
  .else
    rv .set 2 - (by >= 5) - (by >= 10)
  .endif
.endmacro
.macro attr_byte lay, cell
  region_of lay, (cell .mod 8) * 2, (cell / 8) * 2
  ab .set rv
  region_of lay, (cell .mod 8) * 2 + 1, (cell / 8) * 2
  ab .set ab | (rv << 2)
  region_of lay, (cell .mod 8) * 2, (cell / 8) * 2 + 1
  ab .set ab | (rv << 4)
  region_of lay, (cell .mod 8) * 2 + 1, (cell / 8) * 2 + 1
  .byte ab | (rv << 6)
.endmacro
AttrTables:
.repeat 4, L
  .repeat 64, K
    attr_byte L, K
  .endrepeat
.endrepeat
AttrLo:
  .byte <(AttrTables), <(AttrTables + 64), <(AttrTables + 128), <(AttrTables + 192)
AttrHi:
  .byte >(AttrTables), >(AttrTables + 64), >(AttrTables + 128), >(AttrTables + 192)
DemoAttr:
  .res 64, $e4

.segment "VECTORS"
  .word Nmi
  .word Reset
  .word Irq

; Pattern table 0: demo tiles 0-31, font at ASCII codes, rest blank.
; Pattern table 1: every tile solid colour 1 (the colour field).
.macro glyph r0, r1, r2, r3, r4, r5, r6
  .byte r0 << 2, r1 << 2, r2 << 2, r3 << 2, r4 << 2, r5 << 2, r6 << 2, 0
  .res 8, 0
.endmacro
.macro blank count
  .res 16 * (count), 0
.endmacro

.segment "CHR"
  .repeat 32
    .byte $00,$18,$3c,$7e,$ff,$7e,$3c,$18
    .byte $00,$00,$18,$3c,$7e,$3c,$18,$00
  .endrepeat
  blank 14                                                                   ; $20-$2d
  glyph %00000,%00000,%00000,%00000,%00000,%01100,%01100                     ; .
  blank 11                                                                   ; $2f-$39
  glyph %00000,%01100,%01100,%00000,%01100,%01100,%00000                     ; :
  blank 2
  glyph %00000,%00000,%11111,%00000,%11111,%00000,%00000                     ; =
  blank 3
  glyph %01110,%10001,%10001,%11111,%10001,%10001,%10001                     ; A
  glyph %11110,%10001,%10001,%11110,%10001,%10001,%11110                     ; B
  glyph %01110,%10001,%10000,%10000,%10000,%10001,%01110                     ; C
  blank 1                                                                    ; D
  glyph %11111,%10000,%10000,%11110,%10000,%10000,%11111                     ; E
  glyph %11111,%10000,%10000,%11110,%10000,%10000,%10000                     ; F
  glyph %01110,%10001,%10000,%10111,%10001,%10001,%01111                     ; G
  glyph %10001,%10001,%10001,%11111,%10001,%10001,%10001                     ; H
  glyph %01110,%00100,%00100,%00100,%00100,%00100,%01110                     ; I
  blank 2                                                                    ; J K
  glyph %10000,%10000,%10000,%10000,%10000,%10000,%11111                     ; L
  glyph %10001,%11011,%10101,%10101,%10001,%10001,%10001                     ; M
  glyph %10001,%11001,%10101,%10011,%10001,%10001,%10001                     ; N
  glyph %01110,%10001,%10001,%10001,%10001,%10001,%01110                     ; O
  glyph %11110,%10001,%10001,%11110,%10000,%10000,%10000                     ; P
  blank 1                                                                    ; Q
  glyph %11110,%10001,%10001,%11110,%10100,%10010,%10001                     ; R
  glyph %01111,%10000,%10000,%01110,%00001,%00001,%11110                     ; S
  glyph %11111,%00100,%00100,%00100,%00100,%00100,%00100                     ; T
  glyph %10001,%10001,%10001,%10001,%10001,%10001,%01110                     ; U
  glyph %10001,%10001,%10001,%10001,%10001,%01010,%00100                     ; V
  glyph %10001,%10001,%10001,%10101,%10101,%10101,%01010                     ; W
  blank 1                                                                    ; X
  glyph %10001,%10001,%01010,%00100,%00100,%00100,%00100                     ; Y
  glyph %11111,%00001,%00010,%00100,%01000,%10000,%11111                     ; Z
  blank $100 - $5b
  .repeat 256
    .byte $ff,$ff,$ff,$ff,$ff,$ff,$ff,$ff
    .byte $00,$00,$00,$00,$00,$00,$00,$00
  .endrepeat
