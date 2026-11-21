; Isolated DMG Lines scene. Not included in main.asm until the catalog and
; safe VRAM ownership/transition protocol are decided.
;
; LinesLoad: LCD MUST be off. Overwrites $8000-$81FF and BOTH BG maps;
; caller must restore native and colour-field VRAM before showing either.
; Initializes to plane, with the first look change held for 240 steps.
; Clobbers AF, BC, DE and HL; the caller must prepare output before display.
;
; LinesStep: A = requested look (0 plane, nonzero hallway), B = energy byte.
; Call once per displayed frame after input is made. Clobbers AF, BC and HL.
; Prepares wShBGP/LCDC/SCX/SCY for the existing ApplyDisplay, but does not
; touch VRAM, scene selection, input roles, warning, palette gates or debounce.
; Look changes are separated by 240 steps. Scroll is 0..7, moving at most
; one pixel per step. No input changes the palette.

DEF LINES_HOLD EQU 240

SECTION "Lines state", WRAM0
wLinesLook:  ds 1
wLinesHold:  ds 1
wLinesClock: ds 1
wLinesPhase: ds 1

SECTION "Lines code", ROM0
LinesLoad:
  ld hl, $8000
  ld de, LinesTiles
  ld bc, LinesTilesEnd - LinesTiles
  call LinesCopy
  ld hl, $9800
  ld de, LinesPlane
  ld bc, LinesPlaneEnd - LinesPlane
  call LinesCopy
  ld hl, $9C00
  ld de, LinesHallway
  ld bc, LinesHallwayEnd - LinesHallway
  call LinesCopy
  xor a
  ld [wLinesLook], a
  ld [wLinesHold], a
  ld [wLinesClock], a
  ld [wLinesPhase], a
  ret

; Copy BC nonzero bytes from DE to HL; only called with LCD off.
LinesCopy:
.loop
  ld a, [de]
  ld [hli], a
  inc de
  dec bc
  ld a, b
  or c
  jr nz, .loop
  ret

LinesStep:
  ; Normalize the request without altering B (energy).
  or a
  jr z, .request
  ld a, 1
.request
  ld c, a
  ld hl, wLinesHold
  ld a, [hl]
  cp LINES_HOLD
  jr z, .ready
  inc [hl]
.ready
  ld a, [hl]
  cp LINES_HOLD
  jr nz, .speed
  ld a, [wLinesLook]
  cp c
  jr z, .speed
  ld a, c
  ld [wLinesLook], a
  xor a
  ld [hl], a
.speed
  ld c, 8
  ld a, b
  cp 85
  jr c, .tick
  ld c, 4
  cp 170
  jr c, .tick
  ld c, 2
.tick
  ld hl, wLinesClock
  inc [hl]
  ld a, [hl]
  cp c
  jr c, .display
  xor a
  ld [hl], a
  ld hl, wLinesPhase
  inc [hl]
  ld a, [hl]
  cp 14
  jr c, .display
  xor a
  ld [hl], a
.display
  ld a, $E4
  ld [wShBGP], a
  xor a
  ld [wShSCX], a
  ld a, [wLinesPhase]
  cp 8
  jr c, .scroll
  ld b, a
  ld a, 14
  sub b
.scroll
  ld [wShSCY], a
  ld a, [wLinesLook]
  or a
  ld a, $91             ; BG enabled, map $9800, unsigned tiles
  jr z, .map
  ld a, $99             ; map $9C00
.map
  ld [wShLCDC], a
  ret

SECTION "Lines tiles", ROM0
LinesTiles:
  ; Bit flags combine a horizontal, vertical, / and \ stroke. Bit 4
  ; mirrors the vertical stroke. Colour 2 is the only non-background colour.
  FOR line_tile, 0, 32
    FOR line_row, 0, 8
      DEF line_bits = 0
      IF line_tile & 1 && line_row == 3
        REDEF line_bits = line_bits | $FF
      ENDC
      IF line_tile & 2
        IF line_tile & 16
          REDEF line_bits = line_bits | $08
        ELSE
          REDEF line_bits = line_bits | $10
        ENDC
      ENDC
      IF line_tile & 4
        REDEF line_bits = line_bits | (1 << line_row)
      ENDC
      IF line_tile & 8
        REDEF line_bits = line_bits | ($80 >> line_row)
      ENDC
      db 0, line_bits
      PURGE line_bits
    ENDR
  ENDR
LinesTilesEnd:
ASSERT LinesTilesEnd - LinesTiles == 32 * 16

; Construct both complete 32x32 maps at assembly time. Only the first 20
; columns and 19 rows contain artwork; the extra row permits seven-pixel
; scroll without exposing uninitialized cells.
MACRO LINES_MAP
  FOR line_y, 0, 32
    FOR line_x, 0, 32
      DEF line_glyph = 0
      IF line_y < 19 && line_x < 20
        DEF line_left = line_x
        IF line_x >= 10
          REDEF line_left = 19 - line_x
        ENDC
        IF \1 == 0 && line_y >= 4
          ; A mirrored plane: receding horizontal bars and three rays.
          DEF line_edge = 9 - (line_y - 4) * 9 / 14
          IF line_left >= line_edge && (line_y == 5 || line_y == 7 || line_y == 10 || line_y == 14 || line_y == 18)
            REDEF line_glyph = line_glyph | 1
          ENDC
          IF line_left == 9
            REDEF line_glyph = line_glyph | 2
          ENDC
          IF line_left == line_edge || line_left == 9 - (line_y - 4) * 5 / 14 || line_left == 9 - (line_y - 4) * 2 / 14
            REDEF line_glyph = line_glyph | 4
          ENDC
          PURGE line_edge
        ELIF \1 == 1
          ; Five nested frames, joined by mirrored diagonals.
          FOR line_edge, 0, 10, 2
            IF (line_y == line_edge || line_y == 18 - line_edge) && line_left >= line_edge
              REDEF line_glyph = line_glyph | 1
            ENDC
            IF line_left == line_edge && line_y >= line_edge && line_y <= 18 - line_edge
              REDEF line_glyph = line_glyph | 2
            ENDC
          ENDR
          IF line_left <= 8 && (line_left == line_y || line_left == 18 - line_y)
            REDEF line_glyph = line_glyph | 4
          ENDC
        ENDC
        IF line_x >= 10
          ; Reflect / and \; move the vertical stroke to the other centre pixel.
          IF line_glyph & 4
            REDEF line_glyph = (line_glyph & ~4) | 8
          ENDC
          IF line_glyph & 2
            REDEF line_glyph = line_glyph | 16
          ENDC
        ENDC
        PURGE line_left
      ENDC
      db line_glyph
      PURGE line_glyph
    ENDR
  ENDR
ENDM

SECTION "Lines plane map", ROM0
LinesPlane:
  LINES_MAP 0
LinesPlaneEnd:
ASSERT LinesPlaneEnd - LinesPlane == 32 * 32

SECTION "Lines hallway map", ROM0
LinesHallway:
  LINES_MAP 1
LinesHallwayEnd:
ASSERT LinesHallwayEnd - LinesHallway == 32 * 32
