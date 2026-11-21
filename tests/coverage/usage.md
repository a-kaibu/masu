# Use-case coverage

346 game runs, 3917 entry points they call, 480 the tests call.

| kind | used by games | tested | by games weighted | by calls weighted |
|---|---|---|---|---|
| IAMStreamSelect | 3 | 0 | 0.0% | 0.0% |
| IBaseFilter | 9 | 1 | 9.1% | 4.5% |
| IEnumPins | 2 | 0 | 0.0% | 0.0% |
| IMediaSample | 13 | 0 | 0.0% | 0.0% |
| IMemAllocator | 4 | 0 | 0.0% | 0.0% |
| IMemInputPin | 5 | 0 | 0.0% | 0.0% |
| IPin | 10 | 0 | 0.0% | 0.0% |
| api | 3348 | 7 | 0.0% | 0.0% |
| d3d | 255 | 134 | 65.1% | 82.4% |
| d3d9 | 141 | 104 | 82.9% | 99.0% |
| glmissing | 1 | 0 | 0.0% | 0.0% |
| stub | 126 | 0 | 0.0% | 0.0% |

## Called by games, reached by no test (3544)

| entry point | games | calls |
|---|---|---|
| `api:kernel32!GetProcAddress` | 332 | 73535 |
| `api:kernel32!GetCurrentThreadId` | 331 | 8312350 |
| `api:kernel32!MultiByteToWideChar` | 326 | 9571441 |
| `api:kernel32!WideCharToMultiByte` | 321 | 849262 |
| `api:kernel32!GetFileType` | 318 | 20612 |
| `api:kernel32!GetStdHandle` | 317 | 23188 |
| `api:kernel32!SetUnhandledExceptionFilter` | 316 | 576 |
| `api:kernel32!CloseHandle` | 314 | 69749 |
| `api:kernel32!HeapAlloc` | 311 | 1149373 |
| `api:kernel32!GetACP` | 311 | 5289 |
| `api:kernel32!GetEnvironmentStringsW` | 311 | 815 |
| `api:kernel32!GetCPInfo` | 310 | 943 |
| `api:kernel32!FreeEnvironmentStringsW` | 310 | 794 |
| `api:kernel32!HeapFree` | 304 | 864818 |
| `api:kernel32!GetStringTypeW` | 304 | 3152 |
| `api:kernel32!GetCommandLineA` | 303 | 914 |
| `api:kernel32!QueryPerformanceCounter` | 295 | 14006131 |
| `api:kernel32!GetCurrentProcessId` | 295 | 25582 |
| `api:kernel32!ReadFile` | 294 | 2463457 |
| `api:kernel32!InitializeCriticalSection` | 292 | 196333 |
| `api:user32!GetSystemMetrics` | 278 | 3008774 |
| `api:kernel32!GetModuleHandleA` | 278 | 20988 |
| `api:winmm!timeGetTime` | 275 | 78978295 |
| `api:kernel32!GetSystemTimeAsFileTime` | 270 | 17864 |
| `api:kernel32!Sleep` | 268 | 12822860 |
| `api:kernel32!IsProcessorFeaturePresent` | 265 | 844 |
| `api:user32!TranslateMessage` | 264 | 202373 |
| `api:user32!ShowWindow` | 264 | 3034 |
| `api:kernel32!GetModuleFileNameA` | 263 | 11622 |
| `api:kernel32!TlsAlloc` | 262 | 2407 |
| `api:kernel32!TlsSetValue` | 261 | 274569 |
| `api:kernel32!CreateThread` | 261 | 3230 |
| `api:user32!EndPaint` | 251 | 27858 |
| `api:user32!BeginPaint` | 251 | 27857 |
| `api:user32!GetDC` | 245 | 2595142 |
| `api:user32!GetClientRect` | 244 | 288049 |
| `api:kernel32!HeapSize` | 242 | 633421 |
| `api:kernel32!HeapReAlloc` | 237 | 302550 |
| `api:ole32!CoInitialize` | 235 | 456 |
| `api:gdi32!SelectObject` | 232 | 305284 |
| `api:gdi32!GetStockObject` | 231 | 4225 |
| `api:user32!ReleaseDC` | 230 | 2594971 |
| `api:kernel32!GetVersionExA` | 230 | 11436 |
| `api:user32!LoadCursorA` | 229 | 2206 |
| `api:winmm!timeBeginPeriod` | 227 | 2198 |
| `api:kernel32!HeapCreate` | 226 | 528 |
| `api:kernel32!SetFilePointer` | 224 | 627689 |
| `api:kernel32!InitializeCriticalSectionAndSpinCount` | 224 | 8114 |
| `api:sake!IDirectSound8::6` | 223 | 257 |
| `api:user32!GetCursorPos` | 222 | 7313441 |
| `api:sake!IDirectSound8::3` | 221 | 1691 |
| `api:kernel32!WaitForSingleObject` | 219 | 96702741 |
| `api:kernel32!CreateEventA` | 219 | 64486 |
| `api:kernel32!WriteFile` | 218 | 1061792 |
| `api:kernel32!LoadLibraryA` | 217 | 2817 |
| `api:kernel32!SetHandleCount` | 217 | 501 |
| `api:kernel32!GetStartupInfoA` | 216 | 687 |
| `api:user32!SetWindowPos` | 214 | 2890 |
| `api:user32!LoadIconA` | 214 | 353 |
| `api:sake!IDirectSoundBuffer8::11` | 212 | 53979 |

## Called by games, not implemented (127)

| entry point | games | calls |
|---|---|---|
| `stub:imm32!ImmGetContext` | 90 | 689 |
| `stub:imm32!ImmAssociateContext` | 75 | 88 |
| `stub:kernel32!InitializeProcThreadAttributeList` | 38 | 76 |
| `stub:kernel32!DeleteProcThreadAttributeList` | 38 | 38 |
| `stub:imm32!ImmReleaseContext` | 35 | 109 |
| `stub:user32!RegisterTouchWindow` | 35 | 35 |
| `stub:imm32!ImmGetOpenStatus` | 25 | 47 |
| `stub:imm32!ImmGetConversionStatus` | 23 | 66 |
| `stub:dwmapi!DwmSetWindowAttribute` | 22 | 76 |
| `stub:imm32!ImmSetOpenStatus` | 17 | 43 |
| `stub:kernel32!AttachConsole` | 12 | 12 |
| `stub:kernel32!SetConsoleOutputCP` | 11 | 11 |
| `stub:imm32!ImmIsIME` | 10 | 32 |
| `stub:kernel32!SetConsoleCP` | 10 | 10 |
| `stub:dwmapi!DwmEnableComposition` | 9 | 9 |
| `stub:setupapi!SetupDiEnumDeviceInterfaces` | 8 | 5505 |
| `stub:kernel32!GetConsoleScreenBufferInfo` | 8 | 65 |
| `stub:ole32!CoGetApartmentType` | 8 | 19 |
| `stub:uxtheme!RefreshImmersiveColorPolicyState` | 8 | 18 |
| `stub:uxtheme!SetPreferredAppMode` | 8 | 18 |
| `stub:user32!SwapMouseButton` | 8 | 16 |
| `stub:avifil32!AVIFileInit` | 8 | 9 |
| `stub:kernel32!SetConsoleMode` | 8 | 8 |
| `stub:user32!GetThreadDesktop` | 8 | 8 |
| `stub:user32!IsTouchWindow` | 7 | 24 |
| `stub:imm32!ImmSetCompositionFontA` | 7 | 11 |
| `stub:user32!GetClipCursor` | 6 | 47841 |
| `stub:kernel32!SetConsoleTextAttribute` | 6 | 68 |
| `stub:winmm!waveInGetNumDevs` | 6 | 42 |
| `stub:uxtheme!ShouldAppsUseDarkMode` | 6 | 18 |
| `stub:imm32!ImmCreateContext` | 6 | 6 |
| `stub:kernel32!GetConsoleProcessList` | 6 | 6 |
| `stub:kernel32!PowerCreateRequest` | 6 | 6 |
| `stub:kernel32!PowerSetRequest` | 6 | 6 |
| `stub:shell32!ExtractIconExA` | 6 | 6 |
| `stub:shell32!SetCurrentProcessExplicitAppUserModelID` | 6 | 6 |
| `stub:uiautomationcore!UiaLookupId` | 6 | 6 |
| `stub:user32!RegisterPowerSettingNotification` | 6 | 6 |
| `stub:uxtheme!FlushMenuThemes` | 6 | 6 |
| `stub:winhttp!WinHttpGetIEProxyConfigForCurrentUser` | 5 | 13 |
| `stub:crypt32!CertOpenSystemStoreA` | 5 | 5 |
| `stub:imm32!ImmDisableTextFrameService` | 5 | 5 |
| `stub:user32!GetCursorInfo` | 4 | 177778 |
| `glmissing:glDrawBuffers` | 4 | 847 |
| `stub:imm32!ImmNotifyIME` | 4 | 4 |
| `stub:shell32!InitNetworkAddressControl` | 4 | 4 |
| `stub:user32!SetGestureConfig` | 4 | 4 |
| `stub:winmm!joyReleaseCapture` | 4 | 4 |
| `stub:shell32!ExtractIconA` | 3 | 5 |
| `stub:ucrtbase!iswxdigit` | 2 | 5105 |
| `stub:imm32!ImmAssociateContextEx` | 2 | 101 |
| `stub:ucrtbase!iswalnum` | 2 | 63 |
| `stub:imm32!ImmGetCompositionStringW` | 2 | 19 |
| `stub:ucrtbase!logbf` | 2 | 5 |
| `stub:user32!SetMenuInfo` | 2 | 4 |
| `stub:crypt32!CertOpenSystemStoreW` | 2 | 2 |
| `stub:imm32!ImmGetProperty` | 2 | 2 |
| `stub:kernel32!CreateMemoryResourceNotification` | 2 | 2 |
| `stub:kernel32!GetConsoleWindow` | 2 | 2 |
| `stub:kernel32!GetGeoInfoW` | 2 | 2 |
