.segment "HEADER"
  .byte "NES", $1a
  .byte 2
  .byte 1
  .byte $00, $00
  .byte $00, $00, $00, $00, $00, $00, $00, $00

.segment "BSS"
; RAM starts on a page boundary for OAM DMA; the stack uses $0100-$01ff.
Oam:
  .res $100
Frame:
  .res 1

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
  stx Frame
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
  inc Frame
  bit $2002
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
  lda Frame
  lsr a
  lsr a
  lsr a
  lsr a
  lsr a
  tax
  lda ForegroundColors,x
  sta $2007
NmiScroll:
  lda #%10000000
  sta $2000
  lda #$00
  sta $2005
  lda Frame
  lsr a
  sta $2005
  pla
  tax
  pla
  rti

Irq:
  rti

LoadOam:
  lda #$00
  sta $2003
  lda #>Oam
  sta $4014
  rts

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

.segment "VECTORS"
  .word Nmi
  .word Reset
  .word Irq

.segment "CHR"
  .repeat 32
    .byte $00,$18,$3c,$7e,$ff,$7e,$3c,$18
    .byte $00,$00,$18,$3c,$7e,$3c,$18,$00
  .endrepeat
  .repeat 32
    .byte $81,$42,$24,$18,$18,$24,$42,$81
    .byte $00,$81,$c3,$e7,$e7,$c3,$81,$00
  .endrepeat
  .repeat 32
    .byte $ff,$81,$bd,$a5,$a5,$bd,$81,$ff
    .byte $00,$7e,$42,$5a,$5a,$42,$7e,$00
  .endrepeat
  .repeat 160
    .byte $11,$22,$44,$88,$11,$22,$44,$88
    .byte $ee,$dd,$bb,$77,$ee,$dd,$bb,$77
  .endrepeat
