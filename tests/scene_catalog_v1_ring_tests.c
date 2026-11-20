#include "../shared/include/chipviz/scene_catalog_v1_ring.h"

#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define BIT(n) (UINT32_C(1) << (n))
#define EXAMPLE (BIT(0) | BIT(2) | BIT(7) | BIT(11))

static void require(int condition, const char *message) {
  if (!condition) {
    fprintf(stderr, "FAIL: %s\n", message);
    exit(1);
  }
}

static ChipvizCatalogV1Snapshot example(void) {
  ChipvizCatalogV1Snapshot snapshot = {{0}, {0}};
  snapshot.usable[0] = 1;
  snapshot.usable[2] = 7;
  snapshot.usable[7] = 1;
  snapshot.usable[11] = 15;
  snapshot.configured[2] = 2;
  snapshot.configured[11] = 3;
  return snapshot;
}

static void pairs(void) {
  unsigned family, variant;
  for (family = 0; family < 256; family++) {
    for (variant = 0; variant < 256; variant++) {
      int valid = family < 14 && variant <= (family == 2 ? 2u : family == 11 ? 3u : 0u);
      require(chipviz_catalog_v1_pair_valid(family, variant) == valid, "catalog pairs");
    }
  }
  require(!chipviz_catalog_v1_pair_valid(UINT_MAX, 0), "wide family does not alias");
  require(!chipviz_catalog_v1_pair_valid(0, UINT_MAX), "wide variant does not alias");
}

static void preparation(void) {
  ChipvizCatalogV1Snapshot snapshot = example(), bad;
  ChipvizCatalogV1Ring session, before;
  uint32_t ring = UINT32_MAX;
  unsigned family, variant;
  chipviz_catalog_v1_ring_reset(&session);
  before = session;
  require(chipviz_catalog_v1_ring_preview(&snapshot, &ring) == CHIPVIZ_RING_OK &&
          ring == EXAMPLE, "preview example ring");
  require(chipviz_catalog_v1_ring_start(&session, &snapshot, 0, ring) == CHIPVIZ_RING_INVALID,
          "explicit opt-in");
  require(chipviz_catalog_v1_ring_start(&session, &snapshot, 2, ring) == CHIPVIZ_RING_INVALID,
          "opt-in must be exact");
  require(chipviz_catalog_v1_ring_start(&session, &snapshot, 1, ring | BIT(14))
          == CHIPVIZ_RING_INVALID, "unknown confirmed family");
  require(chipviz_catalog_v1_ring_start(&session, &snapshot, 1, ring ^ BIT(2))
          == CHIPVIZ_RING_INVALID, "confirmation must match");
  require(memcmp(&session, &before, sizeof(session)) == 0, "failed starts are atomic");
  require(chipviz_catalog_v1_ring_preview(NULL, &ring) == CHIPVIZ_RING_INVALID &&
          chipviz_catalog_v1_ring_preview(&snapshot, NULL) == CHIPVIZ_RING_INVALID,
          "null preview");
  require(chipviz_catalog_v1_ring_start(NULL, &snapshot, 1, EXAMPLE)
          == CHIPVIZ_RING_INVALID &&
          chipviz_catalog_v1_ring_start(&session, NULL, 1, EXAMPLE) == CHIPVIZ_RING_INVALID,
          "null start");
  memset(&bad, 0, sizeof(bad));
  ring = UINT32_MAX;
  require(chipviz_catalog_v1_ring_preview(&bad, &ring) == CHIPVIZ_RING_INVALID &&
          ring == UINT32_MAX, "empty ring leaves output unchanged");
  require(chipviz_catalog_v1_ring_start(&session, &bad, 1, 0) == CHIPVIZ_RING_INVALID,
          "empty ring cannot start");
  /* Exercise every invalid mask bit and every invalid configured variant. */
  for (family = 0; family < CHIPVIZ_CATALOG_V1_FAMILY_COUNT; family++) {
    for (variant = 0; variant < 32; variant++) {
      if (!chipviz_catalog_v1_pair_valid(family, variant)) {
        bad = snapshot;
        bad.usable[family] |= BIT(variant);
        ring = UINT32_MAX;
        require(chipviz_catalog_v1_ring_preview(&bad, &ring) == CHIPVIZ_RING_INVALID &&
                ring == UINT32_MAX, "invalid usable mask");
        bad = snapshot;
        bad.configured[family] = variant;
        require(chipviz_catalog_v1_ring_preview(&bad, &ring) == CHIPVIZ_RING_INVALID,
                "invalid configured variant");
      }
    }
    bad = snapshot;
    bad.configured[family] = 255;
    require(chipviz_catalog_v1_ring_preview(&bad, &ring) == CHIPVIZ_RING_INVALID,
            "large configured variant");
    bad.configured[family] = 256;
    require(chipviz_catalog_v1_ring_preview(&bad, &ring) == CHIPVIZ_RING_INVALID,
            "configured variant does not alias at 256");
  }
  bad = snapshot;
  bad.usable[2] = 1; /* Configured variant 2 unavailable: no substitution. */
  require(chipviz_catalog_v1_ring_preview(&bad, &ring) == CHIPVIZ_RING_OK &&
          ring == (EXAMPLE ^ BIT(2)), "unavailable default excludes family");
  require(chipviz_catalog_v1_ring_start(&session, &snapshot, 1, EXAMPLE) == CHIPVIZ_RING_OK,
          "confirmed start");
  before = session;
  require(chipviz_catalog_v1_ring_start(&session, &snapshot, 1, EXAMPLE)
          == CHIPVIZ_RING_INVALID && memcmp(&session, &before, sizeof(session)) == 0,
          "cannot replace active session");
}

static void check_request(ChipvizCatalogV1RingStatus status, ChipvizCatalogV1Request request,
                          unsigned family, unsigned variant, const char *message) {
  require(status == CHIPVIZ_RING_OK && request.family == family && request.variant == variant,
          message);
}

static void selection(void) {
  ChipvizCatalogV1Snapshot snapshot = example();
  ChipvizCatalogV1Ring session, before;
  ChipvizCatalogV1Request request, sentinel = {UINT_MAX, UINT_MAX};
  ChipvizCatalogV1RingStatus status;
  chipviz_catalog_v1_ring_reset(&session);
  request = sentinel;
  require(chipviz_catalog_v1_ring_next(&session, &snapshot, 0, 0, &request)
          == CHIPVIZ_RING_INVALID && memcmp(&request, &sentinel, sizeof(request)) == 0,
          "selection before opt-in");
  require(chipviz_catalog_v1_ring_start(&session, &snapshot, 1, EXAMPLE) == CHIPVIZ_RING_OK,
          "start example");
  status = chipviz_catalog_v1_ring_next(&session, &snapshot, 0, 0, &request);
  check_request(status, request, 2, 2, "next 0 to configured 2");
  status = chipviz_catalog_v1_ring_previous(&session, &snapshot, 0, 0, &request);
  check_request(status, request, 11, 3, "previous 0 wraps to 11");
  status = chipviz_catalog_v1_ring_next(&session, &snapshot, 2, 2, &request);
  check_request(status, request, 7, 0, "next 2 requests 7");
  status = chipviz_catalog_v1_ring_next(&session, &snapshot, 11, 3, &request);
  check_request(status, request, 0, 0, "next 11 wraps");
  status = chipviz_catalog_v1_ring_direct(&session, &snapshot, 2, 1, &request);
  check_request(status, request, 2, 1, "direct nonconfigured usable variant");
  status = chipviz_catalog_v1_ring_previous(&session, &snapshot, 7, 0, &request);
  check_request(status, request, 2, 2, "relative restores configuration");
  status = chipviz_catalog_v1_ring_next(&session, &snapshot, 2, 1, &request);
  check_request(status, request, 7, 0, "relative accepts actual nonconfigured variant");
  status = chipviz_catalog_v1_ring_direct(&session, &snapshot, 11, 0, &request);
  check_request(status, request, 11, 0, "direct default variant");
  before = session;
  request = sentinel;
#define REJECT(call) do { \
    require((call) == CHIPVIZ_RING_INVALID, "invalid selection rejected"); \
    require(memcmp(&session, &before, sizeof(session)) == 0 && \
            memcmp(&request, &sentinel, sizeof(request)) == 0, "rejection atomic"); \
  } while (0)
  REJECT(chipviz_catalog_v1_ring_direct(&session, &snapshot, 1, 0, &request));
  REJECT(chipviz_catalog_v1_ring_direct(&session, &snapshot, 14, 0, &request));
  REJECT(chipviz_catalog_v1_ring_direct(&session, &snapshot, UINT_MAX, 0, &request));
  REJECT(chipviz_catalog_v1_ring_direct(&session, &snapshot, 2, 3, &request));
  REJECT(chipviz_catalog_v1_ring_direct(&session, &snapshot, 11, 4, &request));
  REJECT(chipviz_catalog_v1_ring_direct(&session, &snapshot, 0, UINT_MAX, &request));
  REJECT(chipviz_catalog_v1_ring_next(&session, &snapshot, 1, 0, &request));
  REJECT(chipviz_catalog_v1_ring_previous(&session, &snapshot, UINT_MAX, 0, &request));
  REJECT(chipviz_catalog_v1_ring_next(&session, &snapshot, 2, 3, &request));
  REJECT(chipviz_catalog_v1_ring_next(&session, &snapshot, 0, UINT_MAX, &request));
  REJECT(chipviz_catalog_v1_ring_next(&session, NULL, 0, 0, &request));
  REJECT(chipviz_catalog_v1_ring_next(&session, &snapshot, 0, 0, NULL));
#undef REJECT

  chipviz_catalog_v1_ring_reset(&session);
  memset(&snapshot, 0, sizeof(snapshot));
  snapshot.usable[7] = 1;
  require(chipviz_catalog_v1_ring_start(&session, &snapshot, 1, BIT(7)) == CHIPVIZ_RING_OK,
          "singleton start");
  status = chipviz_catalog_v1_ring_next(&session, &snapshot, 7, 0, &request);
  check_request(status, request, 7, 0, "singleton next");
  status = chipviz_catalog_v1_ring_previous(&session, &snapshot, 7, 0, &request);
  check_request(status, request, 7, 0, "singleton previous");
  chipviz_catalog_v1_ring_reset(&session);
  memset(&snapshot, 0, sizeof(snapshot));
  snapshot.usable[2] = 7;
  snapshot.configured[2] = 2;
  require(chipviz_catalog_v1_ring_start(&session, &snapshot, 1, BIT(2)) == CHIPVIZ_RING_OK,
          "singleton multi-variant start");
  status = chipviz_catalog_v1_ring_next(&session, &snapshot, 2, 1, &request);
  check_request(status, request, 2, 1, "singleton next preserves direct variant");
  status = chipviz_catalog_v1_ring_previous(&session, &snapshot, 2, 1, &request);
  check_request(status, request, 2, 1, "singleton previous preserves direct variant");
  chipviz_catalog_v1_ring_reset(&session);
  memset(&snapshot, 0, sizeof(snapshot));
  snapshot.usable[0] = snapshot.usable[11] = 1;
  require(chipviz_catalog_v1_ring_start(&session, &snapshot, 1, BIT(0) | BIT(11))
          == CHIPVIZ_RING_OK, "sparse start");
  status = chipviz_catalog_v1_ring_next(&session, &snapshot, 0, 0, &request);
  check_request(status, request, 11, 0, "sparse next skips 7");
  snapshot.usable[2] = 2; /* Configured 0 remains unavailable. */
  chipviz_catalog_v1_ring_reset(&session);
  require(chipviz_catalog_v1_ring_start(&session, &snapshot, 1, BIT(0) | BIT(11))
          == CHIPVIZ_RING_OK, "non-ring usable variant");
  request = sentinel;
  require(chipviz_catalog_v1_ring_direct(&session, &snapshot, 2, 1, &request)
          == CHIPVIZ_RING_INVALID && memcmp(&request, &sentinel, sizeof(request)) == 0,
          "direct usable variant outside ring rejected");
  snapshot.usable[2] = 1;
  snapshot.configured[2] = 0;
  chipviz_catalog_v1_ring_reset(&session);
  require(chipviz_catalog_v1_ring_start(&session, &snapshot, 1, BIT(0) | BIT(2) | BIT(11))
          == CHIPVIZ_RING_OK, "partial usable mask start");
  request = sentinel;
  require(chipviz_catalog_v1_ring_direct(&session, &snapshot, 2, 1, &request)
          == CHIPVIZ_RING_INVALID && memcmp(&request, &sentinel, sizeof(request)) == 0,
          "catalog-valid unavailable variant rejected");
  require(chipviz_catalog_v1_ring_next(&session, &snapshot, 2, 1, &request)
          == CHIPVIZ_RING_INVALID && memcmp(&request, &sentinel, sizeof(request)) == 0,
          "unavailable current variant rejected");
}

static void invalidation(void) {
  ChipvizCatalogV1Snapshot original = example(), changed;
  ChipvizCatalogV1Ring session;
  ChipvizCatalogV1Request request = {42, 43}, sentinel = request;
  unsigned kind;
  for (kind = 0; kind < 5; kind++) {
    chipviz_catalog_v1_ring_reset(&session);
    require(chipviz_catalog_v1_ring_start(&session, &original, 1, EXAMPLE) == CHIPVIZ_RING_OK,
            "invalidation start");
    changed = original;
    if (kind == 0) changed.usable[2] = 0; /* Ring change. */
    if (kind == 1) changed.configured[2] = 1; /* Ring remains the same. */
    if (kind == 2) changed.usable[2] = 5; /* Direct-variant availability changes. */
    if (kind == 3) changed.usable[1] = 1; /* Newly available family. */
    if (kind == 4) changed.usable[7] = 0x80; /* Malformed change. */
    require(chipviz_catalog_v1_ring_next(&session, &changed, 0, 0, &request)
            == CHIPVIZ_RING_RESTART_REQUIRED, "change latches restart");
    require(memcmp(&request, &sentinel, sizeof(request)) == 0, "invalidation no request");
    require(chipviz_catalog_v1_ring_observe(&session, &original)
            == CHIPVIZ_RING_RESTART_REQUIRED, "reversion cannot revive");
    require(chipviz_catalog_v1_ring_direct(&session, &original, 0, 0, &request)
            == CHIPVIZ_RING_RESTART_REQUIRED, "latched selection rejected");
    require(chipviz_catalog_v1_ring_start(&session, &original, 1, EXAMPLE)
            == CHIPVIZ_RING_INVALID, "restart requires reset");
    chipviz_catalog_v1_ring_reset(&session);
    require(chipviz_catalog_v1_ring_start(&session, &original, 1, EXAMPLE)
            == CHIPVIZ_RING_OK, "explicit reset and reconfirmation");
    changed = original;
    changed.usable[2] = 0;
    require(chipviz_catalog_v1_ring_observe(&session, &changed)
            == CHIPVIZ_RING_RESTART_REQUIRED, "explicit observation detects changes");
  }
  chipviz_catalog_v1_ring_reset(&session);
  require(chipviz_catalog_v1_ring_start(&session, &changed, 1, EXAMPLE)
          == CHIPVIZ_RING_INVALID, "changed ring needs new confirmation");
  require(chipviz_catalog_v1_ring_start(&session, &changed, 1, EXAMPLE ^ BIT(2))
          == CHIPVIZ_RING_OK, "new ring explicitly reconfirmed");
  require(chipviz_catalog_v1_ring_next(&session, &changed, 0, 0, &request)
          == CHIPVIZ_RING_OK && request.family == 7, "reconfirmed ring used after reset");
}

int main(void) {
  pairs();
  preparation();
  selection();
  invalidation();
  puts("scene catalog v1 ring C tests passed");
  return 0;
}
