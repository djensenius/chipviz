#include "chipviz/connection.h"

void chipviz_connection_init(
    ChipvizConnection *connection,
    ChipvizConnectionReceive receive,
    void *context) {
  if (connection == 0) {
    return;
  }

  connection->receive = receive;
  connection->context = context;
  connection->procedural_frame = 0;
}

ChipvizConnectionReceiveStatus chipviz_connection_next_frame(
    ChipvizConnection *connection,
    ChipvizFrame *frame,
    ChipvizConnectionSource *source) {
  ChipvizConnectionReceiveStatus status;

  if (connection == 0 || frame == 0) {
    return CHIPVIZ_CONNECTION_RECEIVE_ERROR;
  }

  if (connection->receive != 0) {
    status = connection->receive(connection->context, frame);
    if (status == CHIPVIZ_CONNECTION_RECEIVE_OK) {
      if (source != 0) {
        *source = CHIPVIZ_CONNECTION_SOURCE_RECEIVER;
      }
      return CHIPVIZ_CONNECTION_RECEIVE_OK;
    }

    if (status == CHIPVIZ_CONNECTION_RECEIVE_ERROR) {
      return status;
    }
  }

  chipviz_frame_make_procedural(connection->procedural_frame, frame);
  connection->procedural_frame++;

  if (source != 0) {
    *source = CHIPVIZ_CONNECTION_SOURCE_PROCEDURAL;
  }

  return CHIPVIZ_CONNECTION_RECEIVE_OK;
}
