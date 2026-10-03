#pragma once
#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#else
using HMODULE = void*;
using DWORD = unsigned long;  // Constants.h uses it for a timeout constant
#endif
