# Use-case coverage

346 game runs, 3947 entry points they call, 1152 the tests call.

| kind | used by games | tested | by games weighted | by calls weighted |
|---|---|---|---|---|
| IAMStreamSelect | 3 | 0 | 0.0% | 0.0% |
| IBaseFilter | 9 | 1 | 9.1% | 4.4% |
| IEnumPins | 2 | 0 | 0.0% | 0.0% |
| IMediaSample | 13 | 0 | 0.0% | 0.0% |
| IMemAllocator | 4 | 0 | 0.0% | 0.0% |
| IMemInputPin | 5 | 0 | 0.0% | 0.0% |
| IPin | 11 | 0 | 0.0% | 0.0% |
| api | 3367 | 340 | 33.3% | 48.2% |
| d3d | 259 | 145 | 69.5% | 83.0% |
| d3d9 | 141 | 118 | 88.5% | 99.3% |
| glmissing | 1 | 0 | 0.0% | 0.0% |
| stub | 132 | 4 | 11.3% | 0.1% |

## Called by games, reached by no test (3210)

| entry point | games | calls |
|---|---|---|
| `api:kernel32!GetCurrentThreadId` | 331 | 9791417 |
| `api:kernel32!GetFileType` | 318 | 17720 |
| `api:kernel32!GetStdHandle` | 317 | 24133 |
| `api:kernel32!GetStringTypeW` | 304 | 2911 |
| `api:kernel32!GetCommandLineA` | 303 | 912 |
| `api:kernel32!GetCurrentProcessId` | 295 | 28356 |
| `api:kernel32!InitializeCriticalSection` | 293 | 204967 |
| `api:winmm!timeGetTime` | 276 | 102218183 |
| `api:kernel32!GetSystemTimeAsFileTime` | 270 | 13907 |
| `api:kernel32!IsProcessorFeaturePresent` | 265 | 842 |
| `api:kernel32!HeapSize` | 242 | 607919 |
| `api:kernel32!HeapReAlloc` | 237 | 312794 |
| `api:kernel32!GetVersionExA` | 230 | 14732 |
| `api:winmm!timeBeginPeriod` | 227 | 2320 |
| `api:kernel32!InitializeCriticalSectionAndSpinCount` | 224 | 8062 |
| `api:sake!IDirectSound8::6` | 224 | 258 |
| `api:user32!GetCursorPos` | 223 | 9256915 |
| `api:kernel32!SetFilePointer` | 223 | 625785 |
| `api:sake!IDirectSound8::3` | 222 | 1722 |
| `api:kernel32!SetHandleCount` | 217 | 499 |
| `api:kernel32!GetStartupInfoA` | 216 | 685 |
| `api:sake!IDirectSoundBuffer8::11` | 213 | 51934 |
| `api:sake!IDirectSoundBuffer8::19` | 213 | 51921 |
| `api:user32!LoadIconA` | 213 | 352 |
| `api:sake!IDirectSoundBuffer8::12` | 212 | 1378 |
| `api:kernel32!LCMapStringW` | 211 | 156016 |
| `api:sake!IDirectSoundBuffer8::15` | 204 | 142388 |
| `api:kernel32!GetProcessHeap` | 203 | 96766 |
| `api:kernel32!FlsSetValue` | 201 | 5885 |
| `api:kernel32!FlsAlloc` | 201 | 434 |
| `api:kernel32!FlsGetValue` | 199 | 7436643 |
| `api:kernel32!EncodePointer` | 197 | 143784 |
| `api:kernel32!SetThreadPriority` | 185 | 82164 |
| `api:sake!IDirectSoundBuffer8::14` | 178 | 220 |
| `api:user32!ScreenToClient` | 176 | 7862031 |
| `api:sake!IDirectSoundBuffer8::13` | 171 | 1025 |
| `api:kernel32!ResumeThread` | 169 | 8611973 |
| `api:kernel32!VirtualAlloc` | 167 | 68336 |
| `api:kernel32!DeleteCriticalSection` | 167 | 56372 |
| `api:kernel32!InterlockedIncrement` | 165 | 7797143 |
| `api:kernel32!GetStartupInfoW` | 162 | 493 |
| `api:kernel32!InterlockedDecrement` | 161 | 515903 |
| `api:winmm!timeGetDevCaps` | 160 | 295 |
| `api:user32!SetCursor` | 154 | 24403 |
| `api:sake!IDirectSoundBuffer8::4` | 153 | 619573 |
| `api:kernel32!GetLocalTime` | 150 | 159086 |
| `api:kernel32!GetModuleFileNameW` | 150 | 1172 |
| `api:kernel32!EnterCriticalSection` | 149 | 174597 |
| `api:kernel32!GetCurrentProcess` | 149 | 18826 |
| `api:sake!IDirectSoundBuffer8::18` | 148 | 982 |
| `api:sake!IDirectSoundBuffer8::2` | 143 | 872 |
| `api:kernel32!GetCommandLineW` | 141 | 704 |
| `api:kernel32!GetTimeZoneInformation` | 141 | 216 |
| `api:kernel32!DecodePointer` | 140 | 148233 |
| `api:kernel32!ResetEvent` | 138 | 204021 |
| `api:kernel32!VirtualFree` | 129 | 26553 |
| `api:kernel32!GetCurrentThread` | 128 | 45462 |
| `api:user32!AdjustWindowRectEx` | 128 | 317 |
| `api:kernel32!GetFileSize` | 127 | 8333 |
| `api:user32!GetAsyncKeyState` | 126 | 11825898 |

## Called by games, not implemented (133)

| entry point | games | calls |
|---|---|---|
| `stub:imm32!ImmGetContext` | 89 | 536 |
| `stub:imm32!ImmAssociateContext` | 73 | 85 |
| `stub:kernel32!InitializeProcThreadAttributeList` | 38 | 76 |
| `stub:kernel32!DeleteProcThreadAttributeList` | 38 | 38 |
| `stub:imm32!ImmReleaseContext` | 34 | 108 |
| `stub:user32!RegisterTouchWindow` | 34 | 34 |
| `stub:imm32!ImmGetOpenStatus` | 25 | 47 |
| `stub:imm32!ImmGetConversionStatus` | 23 | 66 |
| `stub:dwmapi!DwmSetWindowAttribute` | 22 | 91 |
| `stub:imm32!ImmSetOpenStatus` | 17 | 43 |
| `stub:kernel32!AttachConsole` | 12 | 12 |
| `stub:kernel32!SetConsoleOutputCP` | 11 | 11 |
| `stub:imm32!ImmIsIME` | 10 | 32 |
| `stub:kernel32!SetConsoleCP` | 10 | 10 |
| `stub:dwmapi!DwmEnableComposition` | 9 | 9 |
| `stub:setupapi!SetupDiEnumDeviceInterfaces` | 8 | 4992 |
| `stub:kernel32!GetConsoleScreenBufferInfo` | 8 | 54 |
| `stub:ole32!CoGetApartmentType` | 8 | 19 |
| `stub:uxtheme!RefreshImmersiveColorPolicyState` | 8 | 18 |
| `stub:uxtheme!SetPreferredAppMode` | 8 | 18 |
| `stub:user32!SwapMouseButton` | 8 | 16 |
| `stub:avifil32!AVIFileInit` | 8 | 9 |
| `stub:kernel32!SetConsoleMode` | 8 | 8 |
| `stub:user32!GetThreadDesktop` | 8 | 8 |
| `stub:user32!IsTouchWindow` | 7 | 24 |
| `stub:imm32!ImmSetCompositionFontA` | 7 | 11 |
| `stub:kernel32!SetConsoleTextAttribute` | 6 | 72 |
| `stub:winmm!waveInGetNumDevs` | 6 | 33 |
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
| `stub:user32!GetClipCursor` | 5 | 75705 |
| `stub:winhttp!WinHttpGetIEProxyConfigForCurrentUser` | 5 | 8 |
| `stub:imm32!ImmDisableTextFrameService` | 5 | 5 |
| `stub:user32!GetCursorInfo` | 4 | 183187 |
| `glmissing:glDrawBuffers` | 4 | 964 |
| `stub:imm32!ImmNotifyIME` | 4 | 4 |
| `stub:shell32!InitNetworkAddressControl` | 4 | 4 |
| `stub:user32!SetGestureConfig` | 4 | 4 |
| `stub:winmm!joyReleaseCapture` | 4 | 4 |
| `stub:shell32!ExtractIconA` | 3 | 5 |
| `stub:ucrtbase!iswxdigit` | 2 | 5105 |
| `stub:imm32!ImmAssociateContextEx` | 2 | 80 |
| `stub:ucrtbase!iswalnum` | 2 | 63 |
| `stub:ucrtbase!logbf` | 2 | 5 |
| `stub:user32!SetMenuInfo` | 2 | 4 |
| `stub:crypt32!CertOpenSystemStoreW` | 2 | 2 |
| `stub:imm32!ImmGetProperty` | 2 | 2 |
| `stub:kernel32!CreateMemoryResourceNotification` | 2 | 2 |
| `stub:kernel32!GetConsoleWindow` | 2 | 2 |
| `stub:kernel32!GetGeoInfoW` | 2 | 2 |
| `stub:kernel32!GetUserGeoID` | 2 | 2 |
