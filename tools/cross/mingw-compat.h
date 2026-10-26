/* Nur für den Cross-Build unter Linux (MinGW): ältere MinGW-Header kennen die
 * Thread-Power-Throttling-Typen nicht, die ggml (whisper.cpp) unter Windows nutzt.
 * Unter Windows mit MSVC wird diese Datei nicht gebraucht. */
#pragma once
#include <windows.h>
#ifndef THREAD_POWER_THROTTLING_CURRENT_VERSION
#define THREAD_POWER_THROTTLING_CURRENT_VERSION 1
#define THREAD_POWER_THROTTLING_EXECUTION_SPEED 0x1
typedef struct _THREAD_POWER_THROTTLING_STATE {
  ULONG Version;
  ULONG ControlMask;
  ULONG StateMask;
} THREAD_POWER_THROTTLING_STATE;
#endif
