.segment "HEADER"
  .byte "NES", $1a
  .byte 2
  .byte 1
  .byte $00, $00
  .byte $00, $00, $00, $00, $00, $00, $00, $00

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
  bit $2002
WaitVBlank1:
  bit $2002
  bpl WaitVBlank1
WaitVBlank2:
  bit $2002
  bpl WaitVBlank2
  jsr LoadPalette
  jsr LoadNameTable
  lda #%10000000
  sta $2000
  lda #%00011110
  sta $2001
Forever:
  jmp Forever

Nmi:
  inc Frame
  lda #$3f
  sta $2006
  lda #$00
  sta $2006
  lda Frame
  and #$0f
  tax
  lda Palettes,x
  sta $2007
  lda #$00
  sta $2005
  lda Frame
  lsr a
  sta $2005
  rti

Irq:
  rti

LoadPalette:
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
  lda #$20
  sta $2006
  lda #$00
  sta $2006
  ldx #$00
NameLoopA:
  txa
  and #$03
  sta $2007
  inx
  bne NameLoopA
NameLoopB:
  txa
  lsr a
  and #$03
  sta $2007
  inx
  bne NameLoopB
  ldx #$40
AttrLoop:
  lda #$e4
  sta $2007
  dex
  bne AttrLoop
  rts

Frame:
  .byte $00

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
