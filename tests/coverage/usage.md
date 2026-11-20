# Use-case coverage

346 game runs, 3924 entry points they call, 5114 the tests call.

| kind | used by games | tested | by games weighted | by calls weighted |
|---|---|---|---|---|
| IAMStreamSelect | 3 | 1 | 33.3% | 25.0% |
| IBaseFilter | 9 | 9 | 100.0% | 100.0% |
| IEnumPins | 2 | 2 | 100.0% | 100.0% |
| IMediaSample | 13 | 0 | 0.0% | 0.0% |
| IMemAllocator | 4 | 0 | 0.0% | 0.0% |
| IMemInputPin | 5 | 0 | 0.0% | 0.0% |
| IPin | 11 | 8 | 82.5% | 98.0% |
| api | 3352 | 1287 | 71.9% | 81.0% |
| d3d | 259 | 134 | 64.6% | 80.5% |
| d3d9 | 141 | 111 | 86.4% | 98.6% |
| glmissing | 1 | 0 | 0.0% | 0.0% |
| stub | 124 | 33 | 36.0% | 0.6% |

## Called by games, reached by no test (2247)

| entry point | games | calls |
|---|---|---|
| `api:kernel32!GetCurrentThreadId` | 331 | 8270530 |
| `api:kernel32!GetCurrentProcessId` | 295 | 25195 |
| `api:kernel32!IsProcessorFeaturePresent` | 265 | 842 |
| `api:kernel32!InitializeCriticalSectionAndSpinCount` | 224 | 8071 |
| `api:kernel32!SetHandleCount` | 217 | 499 |
| `api:kernel32!GetProcessHeap` | 204 | 116808 |
| `api:sake!IDirectSoundBuffer8::15` | 203 | 180260 |
| `api:kernel32!EncodePointer` | 197 | 144541 |
| `api:user32!ScreenToClient` | 179 | 7791170 |
| `api:kernel32!InterlockedIncrement` | 166 | 8438563 |
| `api:kernel32!InterlockedDecrement` | 162 | 506658 |
| `api:user32!SetCursor` | 152 | 23137 |
| `api:kernel32!GetCurrentProcess` | 149 | 16979 |
| `api:kernel32!DecodePointer` | 140 | 149560 |
| `api:user32!AdjustWindowRectEx` | 129 | 318 |
| `api:kernel32!GetCurrentThread` | 128 | 43169 |
| `api:kernelbase!InitializeCriticalSectionEx` | 111 | 20240 |
| `api:kernelbase!FlsSetValue` | 110 | 10465 |
| `api:kernelbase!FlsAlloc` | 110 | 416 |
| `api:kernelbase!LCMapStringEx` | 109 | 19884 |
| `api:sake!cs_wake` | 106 | 189903 |
| `api:kernelbase!FlsGetValue` | 105 | 1561114 |
| `api:sake!d3d9:2:57` | 104 | 3435235 |
| `api:sake!d3d9:2:43` | 103 | 151659 |
| `api:sake!d3d9:2:17` | 103 | 80026 |
| `api:sake!d3d9:2:65` | 102 | 2774391 |
| `api:sake!d3d9:2:69` | 102 | 2389155 |
| `api:sake!d3d9:1:8` | 102 | 1675 |
| `d3d9:IDirect3D9::GetAdapterDisplayMode` | 102 | 1675 |
| `api:sake!d3d9:5:19` | 98 | 37921 |
| `api:sake!d3d9:5:20` | 98 | 37885 |
| `api:sake!IDirectSoundBuffer8::16` | 97 | 752 |
| `api:kernel32!lstrcpyA` | 96 | 226116 |
| `api:sake!d3d9:2:89` | 89 | 611587 |
| `api:sake!d3d9:2:67` | 88 | 2537871 |
| `api:kernel32!InterlockedExchange` | 82 | 597852 |
| `api:sake!d3d9:1:14` | 80 | 133 |
| `api:user32!RegisterWindowMessageA` | 74 | 232 |
| `api:user32!ShowCursor` | 70 | 455 |
| `api:sake!d3d9:2:47` | 69 | 1166961 |
| `api:shell32!DragAcceptFiles` | 69 | 79 |
| `api:user32!DeleteMenu` | 68 | 1062 |
| `api:kernel32!CompareStringW` | 66 | 614463 |
| `api:sake!d3d11:65:2` | 66 | 280 |
| `api:sake!d3d11:70:5` | 65 | 5880 |
| `api:kernel32!VerifyVersionInfoW` | 65 | 1259 |
| `api:d3d11!D3D11CreateDevice` | 65 | 94 |
| `api:sake!d3d11:70:3` | 64 | 10441 |
| `api:sake!d3d11:70:7` | 64 | 3983 |
| `api:sake!d3d11:70:0` | 64 | 267 |
| `api:kernel32!GetVersionExW` | 64 | 229 |
| `api:sake!d3d11:74:2` | 63 | 305133 |
| `api:user32!PtInRect` | 62 | 483297 |
| `api:sake!d3d11:71:44` | 62 | 67380 |
| `api:sake!d3d11:70:20` | 62 | 702 |
| `api:sake!d3d11:65:8` | 62 | 188 |
| `d3d:65.8` | 62 | 188 |
| `api:sake!d3d11:71:14` | 61 | 276369 |
| `api:kernel32!LocaleNameToLCID` | 61 | 4475 |
| `api:user32!GetKeyboardType` | 61 | 344 |

## Called by games, not implemented (125)

| entry point | games | calls |
|---|---|---|
| `stub:imm32!ImmGetContext` | 89 | 611 |
| `stub:imm32!ImmAssociateContext` | 74 | 86 |
| `stub:kernel32!InitializeProcThreadAttributeList` | 38 | 76 |
| `stub:kernel32!DeleteProcThreadAttributeList` | 38 | 38 |
| `stub:user32!RegisterTouchWindow` | 35 | 35 |
| `stub:imm32!ImmReleaseContext` | 34 | 111 |
| `stub:imm32!ImmGetOpenStatus` | 25 | 50 |
| `stub:imm32!ImmGetConversionStatus` | 23 | 69 |
| `stub:dwmapi!DwmSetWindowAttribute` | 22 | 91 |
| `stub:imm32!ImmSetOpenStatus` | 17 | 43 |
| `stub:kernel32!AttachConsole` | 12 | 12 |
| `stub:kernel32!SetConsoleOutputCP` | 11 | 11 |
| `stub:imm32!ImmIsIME` | 10 | 32 |
| `stub:kernel32!SetConsoleCP` | 10 | 10 |
| `stub:dwmapi!DwmEnableComposition` | 9 | 9 |
| `stub:kernel32!GetConsoleScreenBufferInfo` | 8 | 54 |
| `stub:user32!IsTouchWindow` | 8 | 25 |
| `stub:ole32!CoGetApartmentType` | 8 | 19 |
| `stub:uxtheme!RefreshImmersiveColorPolicyState` | 8 | 18 |
| `stub:uxtheme!SetPreferredAppMode` | 8 | 18 |
| `stub:user32!SwapMouseButton` | 8 | 16 |
| `stub:avifil32!AVIFileInit` | 8 | 9 |
| `stub:kernel32!SetConsoleMode` | 8 | 8 |
| `stub:user32!GetThreadDesktop` | 8 | 8 |
| `stub:setupapi!SetupDiEnumDeviceInterfaces` | 7 | 4992 |
| `stub:imm32!ImmSetCompositionFontA` | 7 | 11 |
| `stub:kernel32!SetConsoleTextAttribute` | 6 | 72 |
| `stub:winmm!waveInGetNumDevs` | 6 | 44 |
| `stub:uxtheme!ShouldAppsUseDarkMode` | 6 | 18 |
| `stub:crypt32!CertOpenSystemStoreA` | 6 | 6 |
| `stub:imm32!ImmCreateContext` | 6 | 6 |
| `stub:kernel32!GetConsoleProcessList` | 6 | 6 |
| `stub:kernel32!PowerCreateRequest` | 6 | 6 |
| `stub:kernel32!PowerSetRequest` | 6 | 6 |
| `stub:shell32!ExtractIconExA` | 6 | 6 |
| `stub:shell32!SetCurrentProcessExplicitAppUserModelID` | 6 | 6 |
| `stub:uiautomationcore!UiaLookupId` | 6 | 6 |
| `stub:user32!RegisterPowerSettingNotification` | 6 | 6 |
| `stub:uxtheme!FlushMenuThemes` | 6 | 6 |
| `stub:user32!GetClipCursor` | 5 | 41634 |
| `stub:winhttp!WinHttpGetIEProxyConfigForCurrentUser` | 5 | 11 |
| `stub:imm32!ImmDisableTextFrameService` | 5 | 5 |
| `stub:user32!GetCursorInfo` | 4 | 145375 |
| `glmissing:glDrawBuffers` | 4 | 840 |
| `stub:imm32!ImmNotifyIME` | 4 | 4 |
| `stub:shell32!InitNetworkAddressControl` | 4 | 4 |
| `stub:user32!SetGestureConfig` | 4 | 4 |
| `stub:winmm!joyReleaseCapture` | 4 | 4 |
| `stub:shell32!ExtractIconA` | 3 | 5 |
| `stub:ucrtbase!iswxdigit` | 2 | 5105 |
| `stub:imm32!ImmAssociateContextEx` | 2 | 92 |
| `stub:ucrtbase!iswalnum` | 2 | 63 |
| `stub:imm32!ImmGetCompositionStringW` | 2 | 8 |
| `stub:ucrtbase!logbf` | 2 | 5 |
| `stub:user32!SetMenuInfo` | 2 | 4 |
| `stub:crypt32!CertOpenSystemStoreW` | 2 | 2 |
| `stub:imm32!ImmGetProperty` | 2 | 2 |
| `stub:kernel32!CreateMemoryResourceNotification` | 2 | 2 |
| `stub:kernel32!GetConsoleWindow` | 2 | 2 |
| `stub:kernel32!GetGeoInfoW` | 2 | 2 |
