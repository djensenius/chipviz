#ifndef CHIPVIZ_CONNECTION_H
#define CHIPVIZ_CONNECTION_H

#include <stdint.h>

#include "chipviz/control_frame.h"

typedef enum {
  CHIPVIZ_CONNECTION_RECEIVE_OK = 0,
  CHIPVIZ_CONNECTION_RECEIVE_NONE,
  CHIPVIZ_CONNECTION_RECEIVE_ERROR
} ChipvizConnectionReceiveStatus;

typedef enum {
  CHIPVIZ_CONNECTION_SOURCE_PROCEDURAL = 0,
  CHIPVIZ_CONNECTION_SOURCE_RECEIVER
} ChipvizConnectionSource;

typedef ChipvizConnectionReceiveStatus (*ChipvizConnectionReceive)(
    void *context,
    ChipvizFrame *frame);

typedef struct {
  ChipvizConnectionReceive receive;
  void *context;
  uint16_t procedural_frame;
} ChipvizConnection;

void chipviz_connection_init(
    ChipvizConnection *connection,
    ChipvizConnectionReceive receive,
    void *context);
ChipvizConnectionReceiveStatus chipviz_connection_next_frame(
    ChipvizConnection *connection,
    ChipvizFrame *frame,
    ChipvizConnectionSource *source);

#endif
