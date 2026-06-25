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
  ldh [$ff40], a
  call LoadTiles
  call LoadMap
  ld a, %11100100
  ldh [$ff47], a
  ld a, %10010001
  ldh [$ff40], a
Loop:
  call WaitVBlank
  ld a, [Frame]
  inc a
  ld [Frame], a
  ldh [$ff42], a
  rrca
  ldh [$ff43], a
  and %00011000
  or %11100100
  ldh [$ff47], a
  jr Loop

WaitVBlank:
.waitVisible
  ldh a, [$ff44]
  cp 144
  jr nc, .waitVisible
.waitVBlank
  ldh a, [$ff44]
  cp 144
  jr c, .waitVBlank
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

SECTION "State", WRAM0
Frame:
  ds 1

SECTION "Tiles", ROM0
Tiles:
  db $00,$00,$18,$18,$3c,$3c,$7e,$7e,$ff,$ff,$7e,$7e,$3c,$3c,$18,$18
  db $81,$81,$42,$42,$24,$24,$18,$18,$18,$18,$24,$24,$42,$42,$81,$81
  db $ff,$00,$81,$7e,$bd,$42,$a5,$5a,$a5,$5a,$bd,$42,$81,$7e,$ff,$00
  db $11,$ee,$22,$dd,$44,$bb,$88,$77,$11,$ee,$22,$dd,$44,$bb,$88,$77
TilesEnd:
