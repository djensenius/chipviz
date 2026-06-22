#include "chipviz/connection.h"

#include <stdio.h>
#include <stdlib.h>

typedef struct {
  ChipvizConnectionReceiveStatus status;
  unsigned int calls;
} FakeReceiver;

static void require(int condition, const char *message) {
  if (!condition) {
    fprintf(stderr, "FAIL: %s\n", message);
    exit(1);
  }
}

static ChipvizConnectionReceiveStatus fake_receive(void *context, ChipvizFrame *frame) {
  FakeReceiver *receiver = (FakeReceiver *)context;
  receiver->calls++;
  chipviz_frame_make_procedural(99, frame);
  frame->scene = 7;
  return receiver->status;
}

static void falls_back_to_procedural_frames(void) {
  ChipvizConnection connection;
  ChipvizFrame frame;
  ChipvizConnectionSource source;

  chipviz_connection_init(&connection, NULL, NULL);
  require(chipviz_connection_next_frame(&connection, &frame, &source) == CHIPVIZ_CONNECTION_RECEIVE_OK, "first procedural frame succeeds");
  require(source == CHIPVIZ_CONNECTION_SOURCE_PROCEDURAL, "procedural source reported");
  require(frame.frame == 0, "first procedural frame index");
  require(chipviz_connection_next_frame(&connection, &frame, &source) == CHIPVIZ_CONNECTION_RECEIVE_OK, "second procedural frame succeeds");
  require(frame.frame == 1, "second procedural frame index");
}

static void receiver_frame_wins(void) {
  ChipvizConnection connection;
  ChipvizFrame frame;
  ChipvizConnectionSource source;
  FakeReceiver receiver = {CHIPVIZ_CONNECTION_RECEIVE_OK, 0};

  chipviz_connection_init(&connection, fake_receive, &receiver);
  require(chipviz_connection_next_frame(&connection, &frame, &source) == CHIPVIZ_CONNECTION_RECEIVE_OK, "receiver frame succeeds");
  require(source == CHIPVIZ_CONNECTION_SOURCE_RECEIVER, "receiver source reported");
  require(receiver.calls == 1, "receiver called once");
  require(frame.frame == 99, "receiver frame used");
  require(frame.scene == 7, "receiver edits preserved");
}

static void receiver_none_falls_back(void) {
  ChipvizConnection connection;
  ChipvizFrame frame;
  ChipvizConnectionSource source;
  FakeReceiver receiver = {CHIPVIZ_CONNECTION_RECEIVE_NONE, 0};

  chipviz_connection_init(&connection, fake_receive, &receiver);
  require(chipviz_connection_next_frame(&connection, &frame, &source) == CHIPVIZ_CONNECTION_RECEIVE_OK, "fallback after none succeeds");
  require(source == CHIPVIZ_CONNECTION_SOURCE_PROCEDURAL, "fallback source reported");
  require(receiver.calls == 1, "receiver attempted");
  require(frame.frame == 0, "fallback frame index");
}

static void receiver_error_propagates(void) {
  ChipvizConnection connection;
  ChipvizFrame frame;
  FakeReceiver receiver = {CHIPVIZ_CONNECTION_RECEIVE_ERROR, 0};

  chipviz_connection_init(&connection, fake_receive, &receiver);
  require(chipviz_connection_next_frame(&connection, &frame, NULL) == CHIPVIZ_CONNECTION_RECEIVE_ERROR, "receiver error propagates");
}

int main(void) {
  falls_back_to_procedural_frames();
  receiver_frame_wins();
  receiver_none_falls_back();
  receiver_error_propagates();
  printf("connection tests passed\n");
  return 0;
}
