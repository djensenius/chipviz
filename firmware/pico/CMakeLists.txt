cmake_minimum_required(VERSION 3.13)

include(pico_sdk_import.cmake OPTIONAL)

project(chipviz_pico_joybus C CXX ASM)
set(CMAKE_C_STANDARD 11)
set(CMAKE_CXX_STANDARD 17)

if(PICO_SDK_VERSION_STRING)
  pico_sdk_init()

  add_executable(chipviz_pico_joybus
    src/main.c
    src/joybus_bridge.c
  )
  target_include_directories(chipviz_pico_joybus PRIVATE include)
  pico_generate_pio_header(chipviz_pico_joybus ${CMAKE_CURRENT_LIST_DIR}/src/joybus_bridge.pio)
  target_link_libraries(chipviz_pico_joybus pico_stdlib hardware_pio)
  pico_enable_stdio_usb(chipviz_pico_joybus 1)
  pico_enable_stdio_uart(chipviz_pico_joybus 0)
  pico_add_extra_outputs(chipviz_pico_joybus)
endif()
