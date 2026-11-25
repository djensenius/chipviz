#ifndef CHIPVIZ_GBA_LINES_FIXTURE_H
#define CHIPVIZ_GBA_LINES_FIXTURE_H

#include <stdint.h>

#define FIXTURE_HEADER_SIZE 32u
#define FIXTURE_EVENTS 13u

typedef void (*fixture_emit_t)(const uint8_t *header, const volatile uint16_t *page,
                               const uint16_t *palette);

/* guarded contains one canary, a full page, and one canary. */
void fixture_run(volatile uint16_t *guarded, fixture_emit_t emit);

#endif
